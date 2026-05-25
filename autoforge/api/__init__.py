from fastapi import APIRouter, Depends
import uuid
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession
from autoforge.database import get_db
from autoforge.models import WorktreeSession
from autoforge.schemas.worktree import WorktreeSessionRead
from autoforge.api import projects, issues, change_requests, reviews, preview, system, webhooks

router = APIRouter()
router.include_router(projects.router, prefix="/projects", tags=["projects"])
router.include_router(issues.router, prefix="/issues", tags=["issues"])
router.include_router(change_requests.router, prefix="/change-requests", tags=["change-requests"])
router.include_router(reviews.router, prefix="/reviews", tags=["reviews"])
router.include_router(preview.router, prefix="/preview", tags=["preview"])
router.include_router(system.router, prefix="/system", tags=["system"])
router.include_router(webhooks.router, prefix="/webhooks", tags=["webhooks"])

# Standalone /worktree-sessions endpoint (avoids exposing all CR routes at this prefix)
_sessions_router = APIRouter()


@_sessions_router.get("", response_model=list[WorktreeSessionRead])
async def list_worktree_sessions(
    cr_id: uuid.UUID | None = None,
    db: AsyncSession = Depends(get_db),
) -> list[WorktreeSession]:
    q = select(WorktreeSession).order_by(WorktreeSession.started_at.desc())
    if cr_id:
        q = q.where(WorktreeSession.change_request_id == cr_id)
    result = await db.scalars(q)
    return list(result.all())


router.include_router(_sessions_router, prefix="/worktree-sessions", tags=["sessions"])
