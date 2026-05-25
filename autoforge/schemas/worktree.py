import uuid
from datetime import datetime
from pydantic import BaseModel


class WorktreeSessionRead(BaseModel):
    id: uuid.UUID
    change_request_id: uuid.UUID
    worktree_path: str | None
    branch_name: str | None
    status: str
    iteration_count: int
    report_content: str | None
    started_at: datetime | None
    completed_at: datetime | None

    model_config = {"from_attributes": True}
