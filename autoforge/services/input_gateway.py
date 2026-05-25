"""
Input Gateway — single entry point for all external inputs.
Applies Layer 1 sanitization, rate limiting, deduplication, and queues analysis.
Design doc §6.1, §4.3 Layer 1.
"""
import hashlib
import uuid as uuid_mod
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy import select

from autoforge.models import IssueEntry, Project
from autoforge.core.security import sanitize_input, hash_ip
from autoforge.core.rate_limit import RateLimiter, RATE_LIMITS
from autoforge.core.redis_client import get_redis
from autoforge.tasks.analysis import run_analysis


def _fingerprint(title: str, description: str | None) -> str:
    text = f"{title.strip().lower()}|{(description or '').strip().lower()}"
    return hashlib.sha256(text.encode()).hexdigest()[:32]


class GatewayError(Exception):
    def __init__(self, code: str, message: str, http_status: int = 400) -> None:
        self.code = code
        self.message = message
        self.http_status = http_status
        super().__init__(message)


async def submit_issue(
    *,
    db: AsyncSession,
    project_id: uuid_mod.UUID,
    source_type: str,
    title: str,
    description: str | None,
    category: str | None,
    severity: str | None,
    source_id: str | None = None,
    source_url: str | None = None,
    client_ip: str = "unknown",
) -> IssueEntry:
    """
    Full input pipeline: validate → rate limit → sanitize → deduplicate → persist → queue.
    """
    # 1. Project existence check
    project = await db.get(Project, project_id)
    if not project:
        raise GatewayError("project_not_found", "Project not found", 404)
    if project.status != "active":
        raise GatewayError("project_paused", "Project is not active", 409)

    # 2. Rate limiting (skip for internal sources)
    if source_type not in ("scan", "monitor"):
        redis = get_redis()
        limiter = RateLimiter(redis)
        limit, window = RATE_LIMITS.get(source_type, RATE_LIMITS["default"])
        rate_key = f"rl:{source_type}:{hash_ip(client_ip)}"
        allowed, remaining = await limiter.check(rate_key, limit, window)
        if not allowed:
            raise GatewayError("rate_limited", "Too many requests", 429)

    # 3. Layer 1: Input sanitization (skip internal)
    if source_type not in ("scan", "monitor"):
        text = f"{title} {description or ''}"
        is_safe, reason = await sanitize_input(text, source_type)
        if not is_safe:
            raise GatewayError("input_rejected", f"Input rejected by security filter: {reason}", 400)

    # 4. Duplicate detection
    fingerprint = _fingerprint(title, description)
    existing = await db.scalar(
        select(IssueEntry).where(
            IssueEntry.project_id == project_id,
            IssueEntry.fingerprint == fingerprint,
            IssueEntry.status.notin_(["rejected", "completed"]),
        )
    )
    if existing:
        raise GatewayError("duplicate", f"Duplicate of issue {existing.id}", 409)

    # 5. Persist
    ip_hash = hash_ip(client_ip) if source_type == "widget" else None
    issue = IssueEntry(
        project_id=project_id,
        source_type=source_type,
        source_id=source_id,
        source_url=source_url,
        title=title,
        description=description,
        category=category,
        severity=severity,
        fingerprint=fingerprint,
        submitter_ip_hash=ip_hash,
        status="pending_analysis",
    )
    db.add(issue)
    await db.commit()
    await db.refresh(issue)

    # 6. Queue analysis task (async, non-blocking)
    run_analysis.apply_async(args=[str(issue.id)], countdown=0)

    return issue
