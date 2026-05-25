"""
Review Portal API — handles the two human review checkpoints.
Review 1: approve/reject issue analysis → creates ChangeRequest
Review 2: approve/reject/revise implementation → merge or re-iterate
"""
import uuid
from datetime import datetime, timezone
from fastapi import APIRouter, Depends, HTTPException, Header
from sqlalchemy.ext.asyncio import AsyncSession
from autoforge.database import get_db
from autoforge.models import IssueEntry, IssueAnalysis, ChangeRequest, AdminDecision
from autoforge.schemas import ReviewDecision
from autoforge.core.concurrency import concurrency_manager
from autoforge.websocket.manager import ws_manager

router = APIRouter()


async def _get_admin_id(x_admin_id: str = Header(...)) -> str:
    """Simple admin identity from header. Replace with proper auth in production."""
    return x_admin_id


@router.post("/issues/{issue_id}/review-1")
async def review_1(
    issue_id: uuid.UUID,
    body: ReviewDecision,
    admin_id: str = Depends(_get_admin_id),
    db: AsyncSession = Depends(get_db),
) -> dict:
    """Review checkpoint 1: approve or reject the analysis report."""
    issue = await db.get(IssueEntry, issue_id)
    if not issue:
        raise HTTPException(status_code=404, detail="Issue not found")
    if issue.status != "pending_review":
        raise HTTPException(status_code=409, detail=f"Issue is in status '{issue.status}', expected 'pending_review'")

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
        # Create ChangeRequest
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
        await ws_manager.broadcast(
            str(issue.project_id),
            {"type": "review_needed", "payload": {"stage": "execution", "cr_id": str(cr.id)}},
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
    """Review checkpoint 2: approve merge, request revision, or reject."""
    cr = await db.get(ChangeRequest, cr_id)
    if not cr:
        raise HTTPException(status_code=404, detail="Change request not found")
    if cr.status != "pending_review_2":
        raise HTTPException(status_code=409, detail=f"CR is in status '{cr.status}'")

    # Layer 3: branch merge requires explicit admin decision here (not automated)
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

    if body.decision == "approved":
        cr.status = "approved"
        await db.commit()
        await concurrency_manager.release_slot()
        await ws_manager.broadcast(
            str(cr.project_id),
            {"type": "worktree_update", "payload": {"cr_id": str(cr_id), "status": "approved"}},
        )
        return {"status": "approved", "action": "merge_to_dev"}

    elif body.decision == "revision":
        cr.status = "executing"
        if body.suggestions:
            existing = cr.admin_suggestions_2 or ""
            cr.admin_suggestions_2 = f"{existing}\n---\n{body.suggestions}".strip()
        await db.commit()
        return {"status": "revision_requested"}

    else:  # rejected
        cr.status = "rejected"
        await db.commit()
        await concurrency_manager.release_slot()
        return {"status": "rejected"}
