import uuid
from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession
from autoforge.database import get_db
from autoforge.models import PreviewEnvironment
from autoforge.schemas import PreviewEnvironmentRead

router = APIRouter()


@router.get("/{project_id}", response_model=list[PreviewEnvironmentRead])
async def list_previews(
    project_id: uuid.UUID, db: AsyncSession = Depends(get_db)
) -> list[PreviewEnvironment]:
    result = await db.scalars(
        select(PreviewEnvironment)
        .where(
            PreviewEnvironment.project_id == project_id,
            PreviewEnvironment.status.notin_(["terminated"]),
        )
        .order_by(PreviewEnvironment.created_at.desc())
    )
    return list(result.all())


@router.post("/{env_id}/restart")
async def restart_preview(env_id: uuid.UUID, db: AsyncSession = Depends(get_db)) -> dict:
    """Allow admin to force full container restart from Review Portal."""
    env = await db.get(PreviewEnvironment, env_id)
    if not env:
        raise HTTPException(status_code=404, detail="Preview environment not found")
    if env.status not in ("ready", "failed"):
        raise HTTPException(status_code=409, detail=f"Cannot restart preview in status '{env.status}'")
    # Actual restart is handled by PreviewOrchestrator service (M5)
    env.status = "building"
    await db.commit()
    return {"status": "restart_queued", "env_id": str(env_id)}
