"""
Claude Code execution task — the heart of AutoForge.
Creates worktree, runs Claude Code, tracks progress, queues preview build.
Design doc §10.2.
"""
import asyncio
import uuid
from datetime import datetime, timezone
from celery import Task

from sqlalchemy.orm import selectinload

from autoforge.tasks.celery_app import celery_app
from autoforge.database import AsyncSessionLocal
from autoforge.models import ChangeRequest, WorktreeSession, IssueEntry, IssueAnalysis, Project
from autoforge.core.concurrency import concurrency_manager
from autoforge.services.worktree_manager import create_worktree, remove_worktree
from autoforge.agents.code_agent import build_prompt, run_claude_code
from autoforge.websocket.manager import ws_manager


def _run_async(coro):
    loop = asyncio.new_event_loop()
    try:
        return loop.run_until_complete(coro)
    finally:
        loop.close()


async def _do_execution(change_request_id: str) -> dict:
    async with AsyncSessionLocal() as db:
        cr = await db.get(ChangeRequest, uuid.UUID(change_request_id))
        if not cr:
            return {"status": "cr_not_found"}

        if cr.status != "pending_execution":
            return {"status": "skipped", "current_status": cr.status}

        # Acquire concurrency slot — signal caller to retry if none available
        acquired = await concurrency_manager.acquire_slot(change_request_id)
        if not acquired:
            cr.status = "queued"
            await db.commit()
            return {"status": "slot_unavailable"}

        # Load related models — use selectinload to avoid lazy-load MissingGreenlet in async context
        from sqlalchemy import select as sa_select
        issue = (await db.scalars(
            sa_select(IssueEntry)
            .where(IssueEntry.id == cr.issue_id)
            .options(selectinload(IssueEntry.analysis))
        )).first()
        project = await db.get(Project, cr.project_id)

        # Load analysis for implementation hints
        analysis_summary = None
        implementation_hints = None
        if issue and issue.analysis:
            analysis_summary = issue.analysis.analysis_summary
            raw = issue.analysis.raw_llm_output or {}
            implementation_hints = raw.get("implementation_hints")

        # Count iterations
        from sqlalchemy import func
        iteration = await db.scalar(
            sa_select(func.count()).where(WorktreeSession.change_request_id == cr.id)
        ) or 0
        iteration += 1

        # Build prompt
        project_config = project.config_snapshot or {}
        admin_suggestions = "\n---\n".join(filter(None, [cr.admin_suggestions_1, cr.admin_suggestions_2]))

        prompt = build_prompt(
            issue_title=issue.title if issue else "未知需求",
            issue_description=issue.description if issue else None,
            analysis_summary=analysis_summary,
            implementation_hints=implementation_hints,
            admin_suggestions=admin_suggestions or None,
            project_config=project_config,
            iteration=iteration,
        )

        # Get repo config
        branch_prefix = project_config.get("branches", {}).get("worktree_prefix", "autoforge/cr-")
        dev_branch = project_config.get("branches", {}).get("dev", "dev")
        repo_path = project.repo_url  # local path when self-hosted

        # Create WorktreeSession
        session = WorktreeSession(
            change_request_id=cr.id,
            status="running",
            prompt_snapshot=prompt,
            iteration_count=iteration,
            started_at=datetime.now(timezone.utc),
        )
        db.add(session)
        cr.status = "executing"
        await db.commit()
        await db.refresh(session)

        # Broadcast execution started
        await ws_manager.broadcast(
            str(cr.project_id),
            {
                "type": "worktree_update",
                "payload": {"cr_id": change_request_id, "status": "executing", "iteration": iteration},
            },
        )

        # Create git worktree
        try:
            worktree_path, branch_name = await create_worktree(
                repo_path=repo_path,
                branch_prefix=branch_prefix,
                cr_id=str(cr.id),
                base_branch=dev_branch,
            )
            session.worktree_path = worktree_path
            session.branch_name = branch_name
            await db.commit()
        except Exception as exc:
            session.status = "failed"
            session.completed_at = datetime.now(timezone.utc)
            cr.status = "failed"
            await db.commit()
            await concurrency_manager.release_slot()
            return {"status": "worktree_creation_failed", "error": str(exc)}

        # Run Claude Code
        exit_code, stdout, stderr = await run_claude_code(
            worktree_path=worktree_path,
            prompt=prompt,
            timeout_seconds=project_config.get("execution_timeout", 1800),
        )

        if exit_code != 0:
            session.status = "failed"
            session.report_content = f"## 执行失败\n\n```\n{stderr}\n```"
            session.completed_at = datetime.now(timezone.utc)
            cr.status = "failed"
            await db.commit()
            await concurrency_manager.release_slot()
            await ws_manager.broadcast(
                str(cr.project_id),
                {"type": "worktree_update", "payload": {"cr_id": change_request_id, "status": "failed"}},
            )
            return {"status": "claude_code_failed", "exit_code": exit_code}

        # Success — extract report from Claude output
        report_content = _extract_report(stdout)
        session.status = "completed"
        session.report_content = report_content
        session.completed_at = datetime.now(timezone.utc)

        # Advance CR to pending review 2
        cr.status = "pending_review_2"
        await concurrency_manager.mark_pending_review()
        await db.commit()

        # Broadcast completion
        await ws_manager.broadcast(
            str(cr.project_id),
            {
                "type": "worktree_update",
                "payload": {
                    "cr_id": change_request_id,
                    "status": "completed",
                    "session_id": str(session.id),
                },
            },
        )

        # Queue preview build (M5)
        from autoforge.tasks.preview import build_preview
        build_preview.apply_async(args=[str(session.id)], countdown=0)

        return {"status": "completed", "session_id": str(session.id)}


def _extract_report(claude_output: str) -> str:
    """Extract the implementation report section from Claude Code output."""
    marker = "## 改动摘要"
    idx = claude_output.find(marker)
    if idx != -1:
        return claude_output[idx:]
    # Fallback: return full output
    return claude_output[:10000]


@celery_app.task(
    name="autoforge.tasks.execution.run_claude_code",
    bind=True,
    max_retries=10,
    time_limit=2000,
    soft_time_limit=1800,
)
def run_claude_code_task(self: Task, change_request_id: str) -> dict:
    result = _run_async(_do_execution(change_request_id))
    if result.get("status") == "slot_unavailable":
        # Slot full — back off and retry; CR stays in "queued" until a slot frees up
        raise self.retry(countdown=60, exc=None)
    return result
