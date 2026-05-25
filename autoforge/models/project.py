import uuid
from datetime import datetime
from sqlalchemy import String, Text, Enum, DateTime, func
from sqlalchemy.dialects.postgresql import UUID, JSONB
from sqlalchemy.orm import Mapped, mapped_column, relationship
from autoforge.database import Base


class Project(Base):
    __tablename__ = "projects"

    id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), primary_key=True, default=uuid.uuid4)
    name: Mapped[str] = mapped_column(String(100), nullable=False)
    slug: Mapped[str] = mapped_column(String(100), unique=True, nullable=False)
    description: Mapped[str] = mapped_column(Text, nullable=True)
    repo_url: Mapped[str] = mapped_column(String(500), nullable=False)
    repo_branch_dev: Mapped[str] = mapped_column(String(100), default="dev")
    repo_branch_main: Mapped[str] = mapped_column(String(100), default="main")
    config_snapshot: Mapped[dict] = mapped_column(JSONB, nullable=True)
    status: Mapped[str] = mapped_column(
        Enum("active", "paused", "archived", name="project_status"),
        default="active",
        nullable=False,
    )
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), server_default=func.now())

    # Relationships
    issues: Mapped[list["IssueEntry"]] = relationship(back_populates="project")
    change_requests: Mapped[list["ChangeRequest"]] = relationship(back_populates="project")
    preview_environments: Mapped[list["PreviewEnvironment"]] = relationship(back_populates="project")
    test_sessions: Mapped[list["TestSession"]] = relationship(back_populates="project")
    admin_decisions: Mapped[list["AdminDecision"]] = relationship(back_populates="project")
