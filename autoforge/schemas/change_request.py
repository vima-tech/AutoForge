import uuid
from datetime import datetime
from pydantic import BaseModel


class ChangeRequestRead(BaseModel):
    id: uuid.UUID
    project_id: uuid.UUID
    issue_id: uuid.UUID
    status: str
    admin_id: str | None
    approved_at: datetime | None
    admin_suggestions_1: str | None
    admin_suggestions_2: str | None
    target_branch: str | None
    iteration_count: int | None = None
    created_at: datetime
    updated_at: datetime

    model_config = {"from_attributes": True}


class ChangeRequestUpdate(BaseModel):
    status: str | None = None
    admin_suggestions_2: str | None = None
