import uuid
from datetime import datetime
from sqlalchemy import String, Text, Enum, DateTime, ForeignKey, func
from sqlalchemy.dialects.postgresql import UUID
from sqlalchemy.orm import Mapped, mapped_column, relationship
from autoforge.database import Base


class AdminDecision(Base):
    __tablename__ = "admin_decisions"

    id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), primary_key=True, default=uuid.uuid4)
    project_id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), ForeignKey("projects.id"), nullable=False)
    issue_id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), ForeignKey("issue_entries.id"), nullable=True)
    change_request_id: Mapped[uuid.UUID] = mapped_column(
        UUID(as_uuid=True), ForeignKey("change_requests.id"), nullable=True
    )
    stage: Mapped[str] = mapped_column(
        Enum("review_1", "review_2", "bug_review", name="decision_stage"),
        nullable=False,
    )
    decision: Mapped[str] = mapped_column(
        Enum("approved", "rejected", "revision", "terminated", "rollback", name="decision_type"),
        nullable=False,
    )
    admin_id: Mapped[str] = mapped_column(String(100), nullable=False)
    suggestions: Mapped[str] = mapped_column(Text, nullable=True)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), server_default=func.now())

    # Relationships
    project: Mapped["Project"] = relationship(back_populates="admin_decisions")
    issue: Mapped["IssueEntry"] = relationship(back_populates="admin_decisions")
    change_request: Mapped["ChangeRequest"] = relationship(back_populates="admin_decisions")
