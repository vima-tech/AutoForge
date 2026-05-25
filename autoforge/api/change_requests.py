import uuid
from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession
from autoforge.database import get_db
from autoforge.models import ChangeRequest
from autoforge.schemas import ChangeRequestRead

router = APIRouter()


@router.get("", response_model=list[ChangeRequestRead])
async def list_change_requests(
    project_id: uuid.UUID | None = None,
    status: str | None = None,
    db: AsyncSession = Depends(get_db),
) -> list[ChangeRequest]:
    q = select(ChangeRequest).order_by(ChangeRequest.created_at.desc())
    if project_id:
        q = q.where(ChangeRequest.project_id == project_id)
    if status:
        q = q.where(ChangeRequest.status == status)
    result = await db.scalars(q)
    return list(result.all())


@router.get("/{cr_id}", response_model=ChangeRequestRead)
async def get_change_request(cr_id: uuid.UUID, db: AsyncSession = Depends(get_db)) -> ChangeRequest:
    cr = await db.get(ChangeRequest, cr_id)
    if not cr:
        raise HTTPException(status_code=404, detail="Change request not found")
    return cr
