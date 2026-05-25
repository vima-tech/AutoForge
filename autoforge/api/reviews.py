"""
Review Portal API — two human checkpoints.
Review 1: approve/reject analysis → creates ChangeRequest
Review 2: approve merge / request revision / reject → triggers merge+destroy or re-iteration
Layer 3 security: branch operations only happen here, after explicit admin decision.
"""
import uuid
from datetime import datetime, timezone
from fastapi import APIRouter, Depends, HTTPException, Header
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from autoforge.database import get_db
from autoforge.models import IssueEntry, ChangeRequest, AdminDecision, WorktreeSession, PreviewEnvironment
from autoforge.schemas import ReviewDecision
from autoforge.core.concurrency import concurrency_manager
from autoforge.websocket.manager import ws_manager

router = APIRouter()


async def _get_admin_id(x_admin_id: str = Header(...)) -> str:
    return x_admin_id


@router.post("/issues/{issue_id}/review-1")
async def review_1(
    issue_id: uuid.UUID,
    body: ReviewDecision,
    admin_id: str = Depends(_get_admin_id),
    db: AsyncSession = Depends(get_db),
) -> dict:
    """Checkpoint 1: approve or reject analysis report."""
    issue = await db.get(IssueEntry, issue_id)
    if not issue:
        raise HTTPException(status_code=404, detail="Issue not found")
    if issue.status != "pending_review":
        raise HTTPException(status_code=409, detail=f"Issue is in '{issue.status}', expected 'pending_review'")

    decision = AdminDecision(
        project_id=issue.project_id,
        issue_id=issue_id,
        stage="review_1",
        decision=body.decision,
        admin_id=admin_id,
        suggestions=body.suggestions,
    )
    db.add(decision)

    if body.decision == "approved":
        issue.status = "approved"
        cr = ChangeRequest(
            project_id=issue.project_id,
            issue_id=issue_id,
            status="pending_execution",
            admin_id=admin_id,
            approved_at=datetime.now(timezone.utc),
            admin_suggestions_1=body.suggestions,
        )
        db.add(cr)
        await db.commit()
        await db.refresh(cr)

        # Queue Claude Code execution
        from autoforge.tasks.execution import run_claude_code_task
        run_claude_code_task.apply_async(args=[str(cr.id)], countdown=0)

        await ws_manager.broadcast(
            str(issue.project_id),
            {"type": "worktree_update", "payload": {"stage": "queued", "cr_id": str(cr.id)}},
        )
        return {"status": "approved", "change_request_id": str(cr.id)}

    else:
        issue.status = "rejected"
        await db.commit()
        return {"status": "rejected"}


@router.post("/change-requests/{cr_id}/review-2")
async def review_2(
    cr_id: uuid.UUID,
    body: ReviewDecision,
    admin_id: str = Depends(_get_admin_id),
    db: AsyncSession = Depends(get_db),
) -> dict:
    """Checkpoint 2: approve merge, request revision, or reject."""
    cr = await db.get(ChangeRequest, cr_id)
    if not cr:
        raise HTTPException(status_code=404, detail="Change request not found")
    if cr.status != "pending_review_2":
        raise HTTPException(status_code=409, detail=f"CR is in '{cr.status}', expected 'pending_review_2'")

    decision = AdminDecision(
        project_id=cr.project_id,
        issue_id=cr.issue_id,
        change_request_id=cr_id,
        stage="review_2",
        decision=body.decision,
        admin_id=admin_id,
        suggestions=body.suggestions,
    )
    db.add(decision)

    # Find active preview environments for this CR's sessions
    sessions = await db.scalars(
        select(WorktreeSession).where(WorktreeSession.change_request_id == cr_id)
    )
    session_ids = [str(s.id) for s in sessions.all()]

    preview_envs = await db.scalars(
        select(PreviewEnvironment).where(
            PreviewEnvironment.worktree_session_id.in_(
                [uuid.UUID(sid) for sid in session_ids]
            ),
            PreviewEnvironment.status != "terminated",
        )
    ) if session_ids else []
    env_ids = [str(e.id) for e in (preview_envs.all() if hasattr(preview_envs, "all") else preview_envs)]

    if body.decision == "approved":
        cr.status = "approved"
        await db.commit()

        # Layer 3: trigger git merge (only here, after explicit admin approval)
        _trigger_merge_async(str(cr.id), env_ids)

        await concurrency_manager.release_slot()
        await ws_manager.broadcast(
            str(cr.project_id),
            {"type": "worktree_update", "payload": {"cr_id": str(cr_id), "status": "merging"}},
        )
        return {"status": "approved", "action": "merge_triggered"}

    elif body.decision == "revision":
        # Append suggestions and re-queue execution
        existing = cr.admin_suggestions_2 or ""
        cr.admin_suggestions_2 = f"{existing}\n---\n{body.suggestions}".strip() if body.suggestions else existing
        cr.status = "pending_execution"
        await db.commit()

        # Destroy current preview (new one will be built after next execution)
        for env_id in env_ids:
            _trigger_destroy_preview(env_id)

        from autoforge.tasks.execution import run_claude_code_task
        run_claude_code_task.apply_async(args=[str(cr_id)], countdown=2)

        return {"status": "revision_requested"}

    else:  # rejected
        cr.status = "rejected"
        await db.commit()

        # Destroy preview and release slot
        for env_id in env_ids:
            _trigger_destroy_preview(env_id)

        await concurrency_manager.release_slot()
        return {"status": "rejected"}


def _trigger_merge_async(cr_id: str, env_ids: list[str]) -> None:
    """Fire-and-forget merge task via Celery."""
    from autoforge.tasks.merge import run_merge
    run_merge.apply_async(args=[cr_id, env_ids], countdown=0)


def _trigger_destroy_preview(env_id: str) -> None:
    from autoforge.tasks.preview import destroy_preview
    destroy_preview.apply_async(args=[env_id], countdown=0)
