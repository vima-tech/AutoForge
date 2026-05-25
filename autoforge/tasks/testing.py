"""
Test Agent tasks.
M7: run_reactive_test — triggered post-merge
M8: run_proactive_scan — scheduled daily by Celery beat
"""
import asyncio
import hashlib
import uuid
from datetime import datetime, timezone
from celery import Task

from autoforge.tasks.celery_app import celery_app
from autoforge.database import AsyncSessionLocal
from autoforge.models import ChangeRequest, Project, TestSession, ScanFinding, IssueEntry
from autoforge.agents.test_agent import run_test_suite, run_quality_checks
from autoforge.websocket.manager import ws_manager


def _run_async(coro):
    loop = asyncio.new_event_loop()
    try:
        return loop.run_until_complete(coro)
    finally:
        loop.close()


def _finding_fingerprint(test_name: str, project_id: str) -> str:
    return hashlib.sha256(f"{project_id}:{test_name}".encode()).hexdigest()[:32]


async def _create_bug_issue(
    db, project_id: uuid.UUID, title: str, description: str, severity: str, test_session_id: uuid.UUID
) -> IssueEntry | None:
    """Auto-create an Issue from a scan finding, with dedup by fingerprint."""
    from sqlalchemy import select
    from autoforge.services.input_gateway import submit_issue, GatewayError

    try:
        issue = await submit_issue(
            db=db,
            project_id=project_id,
            source_type="scan",
            title=title,
            description=description,
            category="Bug",
            severity=severity,
            client_ip="internal",
        )
        return issue
    except GatewayError:
        return None  # Duplicate — already tracked


async def _do_reactive_test(cr_id: str) -> dict:
    async with AsyncSessionLocal() as db:
        cr = await db.get(ChangeRequest, uuid.UUID(cr_id))
        if not cr:
            return {"status": "cr_not_found"}

        project = await db.get(Project, cr.project_id)
        if not project:
            return {"status": "project_not_found"}

        project_config = project.config_snapshot or {}
        test_config = project_config.get("test", {})
        repo_path = project.repo_url

        session = TestSession(
            project_id=project.id,
            session_type="reactive",
            change_request_id=cr.id,
            trigger="merge",
            status="running",
            started_at=datetime.now(timezone.utc),
        )
        db.add(session)
        await db.commit()
        await db.refresh(session)

        all_results = {}
        findings = []

        for suite in ("unit", "integration"):
            suite_config = test_config.get(suite, {})
            cmd = suite_config.get("command")
            if not cmd:
                continue

            timeout = suite_config.get("timeout", 300)
            result = await run_test_suite(command=cmd, cwd=repo_path, timeout=timeout)
            all_results[suite] = {
                "passed": result.passed,
                "failed": result.failed,
                "errors": result.errors,
                "duration": result.duration_seconds,
                "coverage": result.coverage_percent,
            }

            for failure in result.failures:
                fp = _finding_fingerprint(failure["test_name"], str(project.id))
                finding = ScanFinding(
                    test_session_id=session.id,
                    check_type=f"test_{suite}",
                    severity="high",
                    title=f"测试失败：{failure['test_name']}",
                    description=failure["error_message"],
                    file_path=failure.get("file_path"),
                    line_number=failure.get("line_number"),
                    fingerprint=fp,
                )
                db.add(finding)
                findings.append(finding)

        await db.commit()

        # Auto-create issues for failures
        issues_created = []
        for finding in findings:
            issue = await _create_bug_issue(
                db,
                project.id,
                f"[自动发现] 测试回归：{finding.title}",
                finding.description or "",
                "high",
                session.id,
            )
            if issue:
                finding.issue_entry_id = issue.id
                issues_created.append(str(issue.id))

        total_failed = sum(r.get("failed", 0) + r.get("errors", 0) for r in all_results.values())
        status = "passed" if total_failed == 0 else "failed"

        session.status = status
        session.results_json = all_results
        session.issues_created = issues_created
        session.summary = f"{'通过' if status == 'passed' else '失败'}：{total_failed} 个问题"
        session.completed_at = datetime.now(timezone.utc)
        await db.commit()

        if total_failed > 0:
            await ws_manager.broadcast(
                str(project.id),
                {
                    "type": "test_result",
                    "payload": {
                        "cr_id": cr_id,
                        "status": status,
                        "failures": total_failed,
                        "issues_created": issues_created,
                    },
                },
            )

        return {"status": status, "results": all_results, "issues_created": issues_created}


async def _do_proactive_scan() -> dict:
    """Daily proactive scan across all active projects."""
    from sqlalchemy import select

    async with AsyncSessionLocal() as db:
        projects = await db.scalars(
            select(Project).where(Project.status == "active")
        )
        results = {}
        for project in projects.all():
            project_config = project.config_snapshot or {}
            quality_config = project_config.get("quality", {})
            test_config = project_config.get("test", {})
            repo_path = project.repo_url

            session = TestSession(
                project_id=project.id,
                session_type="proactive",
                trigger="scheduled",
                status="running",
                started_at=datetime.now(timezone.utc),
            )
            db.add(session)
            await db.commit()
            await db.refresh(session)

            all_results = {}

            # Full test suite
            for suite in ("unit", "integration"):
                cmd = (test_config.get(suite) or {}).get("command")
                if cmd:
                    result = await run_test_suite(
                        command=cmd,
                        cwd=repo_path,
                        timeout=(test_config.get(suite) or {}).get("timeout", 300),
                    )
                    all_results[f"test_{suite}"] = {
                        "passed": result.passed,
                        "failed": result.failed,
                        "errors": result.errors,
                    }

            # Quality checks
            if quality_config:
                quality_results = await run_quality_checks(
                    quality_config=quality_config, cwd=repo_path
                )
                for check_name, check_result in quality_results.items():
                    all_results[f"quality_{check_name}"] = check_result

            total_issues = sum(
                v.get("failed", 0) + v.get("errors", 0)
                for v in all_results.values()
                if isinstance(v, dict)
            )
            status = "passed" if total_issues == 0 else "failed"
            session.status = status
            session.results_json = all_results
            session.summary = f"巡检{'通过' if status == 'passed' else '发现问题'}: {total_issues} 个"
            session.completed_at = datetime.now(timezone.utc)
            await db.commit()

            results[str(project.id)] = {"status": status, "issues": total_issues}

        return results


@celery_app.task(
    name="autoforge.tasks.testing.run_reactive_test",
    bind=True,
    max_retries=1,
    default_retry_delay=30,
)
def run_reactive_test(self: Task, change_request_id: str) -> dict:
    try:
        return _run_async(_do_reactive_test(change_request_id))
    except Exception as exc:
        raise self.retry(exc=exc)


@celery_app.task(name="autoforge.tasks.testing.run_proactive_scan", bind=True)
def run_proactive_scan(self: Task) -> dict:
    return _run_async(_do_proactive_scan())
