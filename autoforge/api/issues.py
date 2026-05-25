import hashlib
import hmac
import uuid
from fastapi import APIRouter, Depends, HTTPException, Request, Header, status
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession
from autoforge.database import get_db
from autoforge.models import IssueEntry, IssueAnalysis
from autoforge.schemas import IssueEntryCreate, IssueEntryRead, IssueAnalysisRead
from autoforge.services.input_gateway import submit_issue, GatewayError
from autoforge.config import settings

router = APIRouter()


@router.post("", response_model=IssueEntryRead, status_code=status.HTTP_201_CREATED)
async def create_issue(
    body: IssueEntryCreate,
    request: Request,
    db: AsyncSession = Depends(get_db),
) -> IssueEntry:
    client_ip = request.client.host if request.client else "unknown"
    try:
        return await submit_issue(
            db=db,
            project_id=body.project_id,
            source_type=body.source_type,
            title=body.title,
            description=body.description,
            category=body.category,
            severity=body.severity,
            source_id=body.source_id,
            source_url=body.source_url,
            client_ip=client_ip,
        )
    except GatewayError as e:
        raise HTTPException(status_code=e.http_status, detail=e.message)


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


@router.get("/{issue_id}/analysis", response_model=IssueAnalysisRead)
async def get_analysis(issue_id: uuid.UUID, db: AsyncSession = Depends(get_db)) -> IssueAnalysis:
    issue = await db.get(IssueEntry, issue_id)
    if not issue:
        raise HTTPException(status_code=404, detail="Issue not found")
    if not issue.analysis:
        raise HTTPException(status_code=404, detail="Analysis not yet available")
    return issue.analysis


@router.post("/webhook/github")
async def github_webhook(
    request: Request,
    x_hub_signature_256: str = Header(None),
    db: AsyncSession = Depends(get_db),
) -> dict:
    """Receive GitHub issue webhooks (M10)."""
    body = await request.body()

    # Verify signature if secret is configured
    github_secret = getattr(settings, "github_webhook_secret", "")
    if github_secret and x_hub_signature_256:
        expected = "sha256=" + hmac.new(
            github_secret.encode(), body, hashlib.sha256
        ).hexdigest()
        if not hmac.compare_digest(expected, x_hub_signature_256):
            raise HTTPException(status_code=401, detail="Invalid signature")

    import json
    payload = json.loads(body)
    action = payload.get("action")

    if action not in ("opened", "reopened"):
        return {"status": "ignored", "action": action}

    gh_issue = payload.get("issue", {})
    repo = payload.get("repository", {})
    project_id_str = payload.get("autoforge_project_id")  # custom header or payload field

    if not project_id_str:
        return {"status": "ignored", "reason": "no autoforge_project_id in payload"}

    client_ip = request.client.host if request.client else "unknown"
    try:
        issue = await submit_issue(
            db=db,
            project_id=uuid.UUID(project_id_str),
            source_type="github",
            title=gh_issue.get("title", "GitHub Issue"),
            description=gh_issue.get("body"),
            category="Bug" if "bug" in [l.get("name", "") for l in gh_issue.get("labels", [])] else None,
            severity=None,
            source_id=str(gh_issue.get("number")),
            source_url=gh_issue.get("html_url"),
            client_ip=client_ip,
        )
        return {"status": "queued", "issue_id": str(issue.id)}
    except GatewayError as e:
        return {"status": "rejected", "reason": e.message}
