import uuid
from datetime import datetime
from sqlalchemy import String, Text, DateTime, ForeignKey, func
from sqlalchemy.dialects.postgresql import UUID
from sqlalchemy.orm import Mapped, mapped_column, relationship
from autoforge.database import Base


class ChangeRequest(Base):
    __tablename__ = "change_requests"

    id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), primary_key=True, default=uuid.uuid4)
    project_id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), ForeignKey("projects.id"), nullable=False)
    issue_id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), ForeignKey("issue_entries.id"), nullable=False)
    status: Mapped[str] = mapped_column(
        String(50), default="pending_execution", nullable=False
    )
    # Status values: pending_execution | executing | pending_review_2 | approved | rejected | merged
    admin_id: Mapped[str] = mapped_column(String(100), nullable=True)
    approved_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=True)
    admin_suggestions_1: Mapped[str] = mapped_column(Text, nullable=True)
    admin_suggestions_2: Mapped[str] = mapped_column(Text, nullable=True)
    target_branch: Mapped[str] = mapped_column(String(100), nullable=True)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), server_default=func.now())
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), server_default=func.now(), onupdate=func.now()
    )

    # Relationships
    project: Mapped["Project"] = relationship(back_populates="change_requests")
    issue: Mapped["IssueEntry"] = relationship(back_populates="change_request")
    worktree_sessions: Mapped[list["WorktreeSession"]] = relationship(back_populates="change_request")
    admin_decisions: Mapped[list["AdminDecision"]] = relationship(back_populates="change_request")
