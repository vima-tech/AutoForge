import uuid
from datetime import datetime
from sqlalchemy import String, Text, Integer, DateTime, ForeignKey, func
from sqlalchemy.dialects.postgresql import UUID
from sqlalchemy.orm import Mapped, mapped_column, relationship
from autoforge.database import Base


class WorktreeSession(Base):
    __tablename__ = "worktree_sessions"

    id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), primary_key=True, default=uuid.uuid4)
    change_request_id: Mapped[uuid.UUID] = mapped_column(
        UUID(as_uuid=True), ForeignKey("change_requests.id"), nullable=False
    )
    worktree_path: Mapped[str] = mapped_column(String(500), nullable=True)
    branch_name: Mapped[str] = mapped_column(String(200), nullable=True)
    status: Mapped[str] = mapped_column(
        String(50), default="pending", nullable=False
    )
    # Status values: pending | running | completed | failed | cancelled
    prompt_snapshot: Mapped[str] = mapped_column(Text, nullable=True)
    iteration_count: Mapped[int] = mapped_column(Integer, default=1)
    report_content: Mapped[str] = mapped_column(Text, nullable=True)
    started_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=True)
    completed_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=True)

    # Relationships
    change_request: Mapped["ChangeRequest"] = relationship(back_populates="worktree_sessions")
    preview_environment: Mapped["PreviewEnvironment"] = relationship(
        back_populates="worktree_session", uselist=False
    )
