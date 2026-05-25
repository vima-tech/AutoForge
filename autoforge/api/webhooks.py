"""External webhook receivers — GitHub Issues (M10)."""
import hashlib
import hmac
import json
import uuid
from fastapi import APIRouter, Request, HTTPException, Header
from autoforge.database import get_db
from fastapi import Depends
from sqlalchemy.ext.asyncio import AsyncSession
from autoforge.services.input_gateway import submit_issue, GatewayError
from autoforge.config import settings

router = APIRouter()


@router.post("/github")
async def github_webhook(
    request: Request,
    x_hub_signature_256: str = Header(None),
    db: AsyncSession = Depends(get_db),
) -> dict:
    """Receive GitHub issue webhook events."""
    body = await request.body()

    # Verify HMAC signature
    github_secret = getattr(settings, "github_webhook_secret", "")
    if github_secret and x_hub_signature_256:
        expected = "sha256=" + hmac.new(
            github_secret.encode(), body, hashlib.sha256
        ).hexdigest()
        if not hmac.compare_digest(expected, x_hub_signature_256):
            raise HTTPException(status_code=401, detail="Invalid signature")

    payload = json.loads(body)
    action = payload.get("action")
    if action not in ("opened", "reopened"):
        return {"status": "ignored", "action": action}

    gh_issue = payload.get("issue", {})
    project_id_str = payload.get("autoforge_project_id")
    if not project_id_str:
        return {"status": "ignored", "reason": "no autoforge_project_id"}

    labels = [lbl.get("name", "").lower() for lbl in gh_issue.get("labels", [])]
    category = "Bug" if "bug" in labels else (
        "Feature" if "enhancement" in labels or "feature" in labels else None
    )
    severity = "critical" if "critical" in labels else (
        "high" if "high" in labels else (
            "low" if "low" in labels else "medium"
        )
    )

    client_ip = request.client.host if request.client else "unknown"
    try:
        issue = await submit_issue(
            db=db,
            project_id=uuid.UUID(project_id_str),
            source_type="github",
            title=gh_issue.get("title", "GitHub Issue"),
            description=gh_issue.get("body"),
            category=category,
            severity=severity,
            source_id=str(gh_issue.get("number")),
            source_url=gh_issue.get("html_url"),
            client_ip=client_ip,
        )
        return {"status": "queued", "issue_id": str(issue.id)}
    except GatewayError as e:
        return {"status": "rejected", "reason": e.message}


@router.post("/monitor")
async def monitor_webhook(
    request: Request,
    db: AsyncSession = Depends(get_db),
) -> dict:
    """Receive monitoring alerts (Sentry, Grafana, etc.)."""
    payload = await request.json()
    project_id_str = payload.get("project_id")
    if not project_id_str:
        raise HTTPException(status_code=400, detail="project_id required")

    title = payload.get("title") or payload.get("alert_name") or "监控告警"
    description = payload.get("description") or payload.get("message")
    severity = payload.get("severity", "high")

    client_ip = request.client.host if request.client else "unknown"
    try:
        issue = await submit_issue(
            db=db,
            project_id=uuid.UUID(project_id_str),
            source_type="monitor",
            title=f"[监控] {title}",
            description=description,
            category="Bug",
            severity=severity,
            source_url=payload.get("url"),
            client_ip=client_ip,
        )
        return {"status": "queued", "issue_id": str(issue.id)}
    except GatewayError as e:
        return {"status": "rejected", "reason": e.message}
