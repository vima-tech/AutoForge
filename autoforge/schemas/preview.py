import uuid
from datetime import datetime
from pydantic import BaseModel


class PreviewEnvironmentRead(BaseModel):
    id: uuid.UUID
    project_id: uuid.UUID
    env_type: str
    worktree_session_id: uuid.UUID | None
    container_id: str | None
    preview_url: str | None
    db_snapshot_name: str | None
    status: str
    created_at: datetime
    ready_at: datetime | None
    terminated_at: datetime | None

    model_config = {"from_attributes": True}
