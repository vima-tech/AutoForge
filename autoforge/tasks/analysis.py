"""
Analysis Agent Celery task.
Runs after an issue enters the pipeline, produces IssueAnalysis and advances status.
"""
import asyncio
import uuid
from celery import Task
from sqlalchemy import select

from autoforge.tasks.celery_app import celery_app
from autoforge.database import AsyncSessionLocal
from autoforge.models import IssueEntry, IssueAnalysis
from autoforge.agents.analysis import analyze_issue
from autoforge.websocket.manager import ws_manager


def _run_async(coro):
    loop = asyncio.new_event_loop()
    try:
        return loop.run_until_complete(coro)
    finally:
        loop.close()


async def _do_analysis(issue_id: str) -> dict:
    async with AsyncSessionLocal() as db:
        issue = await db.get(IssueEntry, uuid.UUID(issue_id))
        if not issue:
            return {"status": "issue_not_found"}

        if issue.status != "pending_analysis":
            return {"status": "skipped", "current_status": issue.status}

        # Gather existing issues for duplicate detection (recent 50)
        recent = await db.scalars(
            select(IssueEntry)
            .where(
                IssueEntry.project_id == issue.project_id,
                IssueEntry.id != issue.id,
                IssueEntry.status.notin_(["rejected"]),
            )
            .order_by(IssueEntry.created_at.desc())
            .limit(50)
        )
        existing_summary = "\n".join(
            f"- [{r.id}] {r.title} (fingerprint: {r.fingerprint})"
            for r in recent.all()
        ) or "无"

        # Run analysis agent
        result = await analyze_issue(
            title=issue.title,
            description=issue.description,
            source_type=issue.source_type,
            category=issue.category,
            severity=issue.severity,
            existing_issues_summary=existing_summary,
        )

        # Resolve duplicate_of UUID
        dup_fp = result.get("duplicate_of_fingerprint")
        duplicate_of_id = None
        if dup_fp:
            dup_issue = await db.scalar(
                select(IssueEntry).where(
                    IssueEntry.project_id == issue.project_id,
                    IssueEntry.fingerprint == dup_fp,
                )
            )
            if dup_issue:
                duplicate_of_id = dup_issue.id

        # Persist analysis
        analysis = IssueAnalysis(
            issue_id=issue.id,
            authenticity_score=result.get("authenticity_score", 1.0),
            feasibility_score=result.get("feasibility_score"),
            priority_suggestion=result.get("priority_suggestion"),
            category_suggestion=result.get("category_suggestion"),
            severity_suggestion=result.get("severity_suggestion"),
            duplicate_of=duplicate_of_id,
            affected_modules={"modules": result.get("affected_modules", [])},
            analysis_summary=result.get("analysis_summary"),
            raw_llm_output=result.get("raw_llm_output"),
        )
        db.add(analysis)

        # Update issue status to pending_review
        issue.status = "pending_review"
        if result.get("priority_suggestion"):
            issue.priority = result["priority_suggestion"]

        await db.commit()

        # Broadcast WebSocket notification
        await ws_manager.broadcast(
            str(issue.project_id),
            {
                "type": "review_needed",
                "payload": {
                    "stage": "review_1",
                    "issue_id": str(issue.id),
                    "title": issue.title,
                    "severity": result.get("severity_suggestion"),
                },
            },
        )

        return {"status": "completed", "issue_id": issue_id}


@celery_app.task(
    name="autoforge.tasks.analysis.run_analysis",
    bind=True,
    max_retries=2,
    default_retry_delay=30,
)
def run_analysis(self: Task, issue_id: str) -> dict:
    try:
        return _run_async(_do_analysis(issue_id))
    except Exception as exc:
        raise self.retry(exc=exc)
