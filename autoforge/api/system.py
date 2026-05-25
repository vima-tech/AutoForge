from fastapi import APIRouter, Depends, HTTPException
from pydantic import BaseModel, Field
from sqlalchemy import select, func, and_
from sqlalchemy.ext.asyncio import AsyncSession
from autoforge.database import get_db
from autoforge.core.concurrency import concurrency_manager
from autoforge.models import AdminDecision, ChangeRequest, PreviewEnvironment, IssueEntry, WorktreeSession

router = APIRouter()


class ConcurrencyConfigUpdate(BaseModel):
    max_slots: int = Field(ge=1, le=50)
    pause_threshold: int = Field(ge=5, le=500)


@router.post("/config")
async def update_concurrency_config(body: ConcurrencyConfigUpdate) -> dict:
    """Hot-update concurrency limits (in-process; restart reverts to env defaults)."""
    await concurrency_manager.update_config(
        max_slots=body.max_slots,
        pause_threshold=body.pause_threshold,
    )
    return {"ok": True, "max_slots": body.max_slots, "pause_threshold": body.pause_threshold}


@router.get("/status")
async def system_status() -> dict:
    """System health and concurrency status for Review Portal dashboard."""
    return {
        "status": "ok",
        "concurrency": await concurrency_manager.status_snapshot(),
    }


@router.get("/health")
async def health() -> dict:
    return {"status": "ok"}


@router.get("/metrics")
async def system_metrics(
    project_id: str | None = None,
    db: AsyncSession = Depends(get_db),
) -> dict:
    """
    KPI metrics from design doc §15.
    Aggregates from admin_decisions and related tables.
    """
    # Filter by project if specified
    def project_filter(model):
        if project_id:
            import uuid
            return [model.project_id == uuid.UUID(project_id)]
        return []

    # 1. Average time from issue creation to merge (minutes)
    merged_crs = await db.execute(
        select(
            IssueEntry.created_at.label("created"),
            ChangeRequest.updated_at.label("merged"),
        )
        .join(ChangeRequest, ChangeRequest.issue_id == IssueEntry.id)
        .where(
            ChangeRequest.status == "merged",
            *project_filter(ChangeRequest),
        )
        .limit(1000)
    )
    merged_rows = merged_crs.all()
    if merged_rows:
        avg_minutes = sum(
            (r.merged - r.created).total_seconds() / 60
            for r in merged_rows
            if r.merged and r.created
        ) / len(merged_rows)
    else:
        avg_minutes = None

    # 2. First-pass approval rate (approved at iteration 1)
    total_review2 = await db.scalar(
        select(func.count()).select_from(AdminDecision)
        .where(AdminDecision.stage == "review_2", *project_filter(AdminDecision))
    )
    first_pass = await db.scalar(
        select(func.count()).select_from(AdminDecision)
        .join(ChangeRequest, ChangeRequest.id == AdminDecision.change_request_id)
        .join(WorktreeSession, WorktreeSession.change_request_id == ChangeRequest.id)
        .where(
            AdminDecision.stage == "review_2",
            AdminDecision.decision == "approved",
            WorktreeSession.iteration_count == 1,
            *project_filter(AdminDecision),
        )
    )
    first_pass_rate = (first_pass / total_review2) if total_review2 else None

    # 3. Preview startup success rate
    total_previews = await db.scalar(
        select(func.count()).select_from(PreviewEnvironment)
        .where(
            PreviewEnvironment.status.in_(["ready", "failed"]),
            *project_filter(PreviewEnvironment),
        )
    )
    ready_previews = await db.scalar(
        select(func.count()).select_from(PreviewEnvironment)
        .where(
            PreviewEnvironment.status == "ready",
            *project_filter(PreviewEnvironment),
        )
    )
    preview_success_rate = (ready_previews / total_previews) if total_previews else None

    # 4. Needs action count
    pending_review1 = await db.scalar(
        select(func.count()).select_from(IssueEntry)
        .where(IssueEntry.status == "pending_review", *project_filter(IssueEntry))
    )
    pending_review2 = await db.scalar(
        select(func.count()).select_from(ChangeRequest)
        .where(ChangeRequest.status == "pending_review_2", *project_filter(ChangeRequest))
    )

    return {
        "avg_time_to_merge_minutes": round(avg_minutes, 1) if avg_minutes else None,
        "avg_time_to_merge_target": 120,
        "first_pass_approval_rate": round(first_pass_rate, 3) if first_pass_rate else None,
        "first_pass_target": 0.60,
        "preview_success_rate": round(preview_success_rate, 3) if preview_success_rate else None,
        "preview_success_target": 0.99,
        "pending_review_1": pending_review1 or 0,
        "pending_review_2": pending_review2 or 0,
        "total_reviews_completed": total_review2 or 0,
    }
