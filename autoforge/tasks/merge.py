"""Merge task: runs after admin approves review 2. Layer 3 of security."""
import asyncio
import uuid
from autoforge.tasks.celery_app import celery_app
from autoforge.database import AsyncSessionLocal
from autoforge.models import ChangeRequest, IssueEntry, WorktreeSession, Project
from autoforge.services.worktree_manager import merge_worktree_to_dev, remove_worktree
from autoforge.websocket.manager import ws_manager


def _run_async(coro):
    loop = asyncio.new_event_loop()
    try:
        return loop.run_until_complete(coro)
    finally:
        loop.close()


async def _do_merge(cr_id: str, env_ids: list[str]) -> dict:
    async with AsyncSessionLocal() as db:
        cr = await db.get(ChangeRequest, uuid.UUID(cr_id))
        if not cr:
            return {"status": "cr_not_found"}

        project = await db.get(Project, cr.project_id)
        if not project:
            return {"status": "project_not_found"}

        project_config = project.config_snapshot or {}
        dev_branch = project_config.get("branches", {}).get("dev", "dev")

        # Find latest completed worktree session
        from sqlalchemy import select
        session = await db.scalar(
            select(WorktreeSession)
            .where(
                WorktreeSession.change_request_id == cr.id,
                WorktreeSession.status == "completed",
            )
            .order_by(WorktreeSession.started_at.desc())
        )

        if not session or not session.branch_name:
            return {"status": "no_completed_session"}

        repo_path = project.repo_url  # local path for self-hosted

        try:
            await merge_worktree_to_dev(
                repo_path=repo_path,
                branch_name=session.branch_name,
                dev_branch=dev_branch,
            )
        except Exception as exc:
            cr.status = "merge_conflict"
            await db.commit()
            await ws_manager.broadcast(
                str(cr.project_id),
                {
                    "type": "worktree_update",
                    "payload": {"cr_id": cr_id, "status": "merge_conflict", "error": str(exc)},
                },
            )
            return {"status": "merge_failed", "error": str(exc)}

        # Cleanup worktree
        try:
            await remove_worktree(
                repo_path=repo_path,
                worktree_path=session.worktree_path or "",
                branch_name=session.branch_name,
            )
        except Exception as exc:
            pass  # Cleanup failure is non-critical

        # Destroy preview environments
        for env_id in env_ids:
            from autoforge.tasks.preview import destroy_preview
            destroy_preview.apply_async(args=[env_id], countdown=0)

        # Update statuses — also mark the source issue as completed
        cr.status = "merged"
        session.status = "completed"
        issue = await db.get(IssueEntry, cr.issue_id)
        if issue:
            issue.status = "completed"
        await db.commit()

        await ws_manager.broadcast(
            str(cr.project_id),
            {"type": "worktree_update", "payload": {"cr_id": cr_id, "status": "merged"}},
        )

        # Trigger reactive test
        from autoforge.tasks.testing import run_reactive_test
        run_reactive_test.apply_async(args=[cr_id], countdown=5)

        return {"status": "merged"}


@celery_app.task(name="autoforge.tasks.merge.run_merge", bind=True, max_retries=1)
def run_merge(self, cr_id: str, env_ids: list[str]) -> dict:
    try:
        return _run_async(_do_merge(cr_id, env_ids))
    except Exception as exc:
        raise self.retry(exc=exc, countdown=10)
