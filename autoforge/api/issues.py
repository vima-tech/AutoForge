import uuid
import hashlib
from fastapi import APIRouter, Depends, HTTPException, Request, status
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession
from autoforge.database import get_db
from autoforge.models import Project, IssueEntry
from autoforge.schemas import IssueEntryCreate, IssueEntryRead
from autoforge.core.security import sanitize_input, hash_ip

router = APIRouter()


def _fingerprint(title: str, description: str | None) -> str:
    text = f"{title}|{description or ''}"
    return hashlib.sha256(text.encode()).hexdigest()[:32]


@router.post("", response_model=IssueEntryRead, status_code=status.HTTP_201_CREATED)
async def create_issue(
    body: IssueEntryCreate,
    request: Request,
    db: AsyncSession = Depends(get_db),
) -> IssueEntry:
    project = await db.get(Project, body.project_id)
    if not project:
        raise HTTPException(status_code=404, detail="Project not found")

    # Layer 1: sanitize external input
    if body.source_type not in ("scan", "monitor"):
        text_to_check = f"{body.title} {body.description or ''}"
        is_safe, reason = await sanitize_input(text_to_check, body.source_type)
        if not is_safe:
            raise HTTPException(status_code=400, detail=f"Input rejected by security filter: {reason}")

    fingerprint = _fingerprint(body.title, body.description)

    # Duplicate detection
    existing = await db.scalar(
        select(IssueEntry).where(
            IssueEntry.project_id == body.project_id,
            IssueEntry.fingerprint == fingerprint,
            IssueEntry.status.notin_(["rejected", "completed"]),
        )
    )
    if existing:
        raise HTTPException(status_code=409, detail=f"Duplicate issue detected: {existing.id}")

    # Hash submitter IP (privacy)
    client_ip = request.client.host if request.client else "unknown"
    ip_hash = hash_ip(client_ip) if body.source_type == "widget" else None

    issue = IssueEntry(
        **body.model_dump(),
        fingerprint=fingerprint,
        submitter_ip_hash=ip_hash,
        status="pending_analysis",
    )
    db.add(issue)
    await db.commit()
    await db.refresh(issue)
    return issue


@router.get("", response_model=list[IssueEntryRead])
async def list_issues(
    project_id: uuid.UUID | None = None,
    status: str | None = None,
    db: AsyncSession = Depends(get_db),
) -> list[IssueEntry]:
    q = select(IssueEntry).order_by(IssueEntry.created_at.desc())
    if project_id:
        q = q.where(IssueEntry.project_id == project_id)
    if status:
        q = q.where(IssueEntry.status == status)
    result = await db.scalars(q)
    return list(result.all())


@router.get("/{issue_id}", response_model=IssueEntryRead)
async def get_issue(issue_id: uuid.UUID, db: AsyncSession = Depends(get_db)) -> IssueEntry:
    issue = await db.get(IssueEntry, issue_id)
    if not issue:
        raise HTTPException(status_code=404, detail="Issue not found")
    return issue
