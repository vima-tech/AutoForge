import uuid
from datetime import datetime
from typing import Literal
from pydantic import BaseModel, Field


class IssueEntryCreate(BaseModel):
    project_id: uuid.UUID
    source_type: Literal["widget", "admin", "github", "monitor", "scan"]
    source_id: str | None = None
    source_url: str | None = None
    title: str = Field(..., min_length=1, max_length=500)
    description: str | None = None
    category: Literal["Bug", "Feature", "Improvement", "Security", "Debt"] | None = None
    severity: Literal["critical", "high", "medium", "low"] | None = None


class IssueEntryRead(BaseModel):
    id: uuid.UUID
    project_id: uuid.UUID
    source_type: str
    title: str
    description: str | None
    category: str | None
    severity: str | None
    priority: int | None
    status: str
    fingerprint: str | None
    created_at: datetime
    updated_at: datetime

    model_config = {"from_attributes": True}


class IssueAnalysisRead(BaseModel):
    id: uuid.UUID
    issue_id: uuid.UUID
    authenticity_score: float
    feasibility_score: float | None
    priority_suggestion: int | None
    category_suggestion: str | None
    severity_suggestion: str | None
    duplicate_of: uuid.UUID | None
    affected_modules: dict | None
    analysis_summary: str | None
    created_at: datetime

    model_config = {"from_attributes": True}
