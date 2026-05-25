import uuid
from datetime import datetime
from sqlalchemy import String, Enum, DateTime, ForeignKey, func
from sqlalchemy.dialects.postgresql import UUID
from sqlalchemy.orm import Mapped, mapped_column, relationship
from autoforge.database import Base


class PreviewEnvironment(Base):
    __tablename__ = "preview_environments"

    id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), primary_key=True, default=uuid.uuid4)
    project_id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), ForeignKey("projects.id"), nullable=False)
    env_type: Mapped[str] = mapped_column(
        Enum("worktree", "dev", "main", name="preview_env_type"),
        nullable=False,
    )
    worktree_session_id: Mapped[uuid.UUID] = mapped_column(
        UUID(as_uuid=True), ForeignKey("worktree_sessions.id"), nullable=True
    )
    container_id: Mapped[str] = mapped_column(String(200), nullable=True)
    preview_url: Mapped[str] = mapped_column(String(500), nullable=True)
    db_snapshot_name: Mapped[str] = mapped_column(String(200), nullable=True)
    status: Mapped[str] = mapped_column(
        Enum("pending", "building", "ready", "failed", "terminated", name="preview_status"),
        default="pending",
        nullable=False,
    )
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), server_default=func.now())
    ready_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=True)
    terminated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=True)
    # Privacy extension fields
    data_masked_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=True)
    mask_policy_version: Mapped[str] = mapped_column(String(50), nullable=True)

    # Relationships
    project: Mapped["Project"] = relationship(back_populates="preview_environments")
    worktree_session: Mapped["WorktreeSession"] = relationship(back_populates="preview_environment")
