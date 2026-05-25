import uuid
from datetime import datetime
from typing import Literal
from pydantic import BaseModel, Field


class ProjectCreate(BaseModel):
    name: str = Field(..., min_length=1, max_length=100)
    slug: str = Field(..., min_length=1, max_length=100, pattern=r"^[a-z0-9-]+$")
    description: str | None = None
    repo_url: str
    repo_branch_dev: str = "dev"
    repo_branch_main: str = "main"
    config_snapshot: dict | None = None


class ProjectUpdate(BaseModel):
    name: str | None = None
    description: str | None = None
    status: Literal["active", "paused", "archived"] | None = None
    config_snapshot: dict | None = None


class ProjectRead(BaseModel):
    id: uuid.UUID
    name: str
    slug: str
    description: str | None
    repo_url: str
    repo_branch_dev: str
    repo_branch_main: str
    status: str
    created_at: datetime

    model_config = {"from_attributes": True}
