import uuid
from datetime import datetime
from sqlalchemy import String, Text, Enum, DateTime, Float, Integer, ForeignKey, func
from sqlalchemy.dialects.postgresql import UUID, JSONB
from sqlalchemy.orm import Mapped, mapped_column, relationship
from autoforge.database import Base


class IssueEntry(Base):
    __tablename__ = "issue_entries"

    id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), primary_key=True, default=uuid.uuid4)
    project_id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), ForeignKey("projects.id"), nullable=False)
    source_type: Mapped[str] = mapped_column(
        Enum("widget", "admin", "github", "monitor", "scan", name="issue_source_type"),
        nullable=False,
    )
    source_id: Mapped[str] = mapped_column(String(500), nullable=True)
    source_url: Mapped[str] = mapped_column(String(500), nullable=True)
    title: Mapped[str] = mapped_column(String(500), nullable=False)
    description: Mapped[str] = mapped_column(Text, nullable=True)
    category: Mapped[str] = mapped_column(
        Enum("Bug", "Feature", "Improvement", "Security", "Debt", name="issue_category"),
        nullable=True,
    )
    severity: Mapped[str] = mapped_column(
        Enum("critical", "high", "medium", "low", name="issue_severity"),
        nullable=True,
    )
    priority: Mapped[int] = mapped_column(Integer, nullable=True)
    status: Mapped[str] = mapped_column(
        Enum(
            "pending_analysis",
            "pending_review",
            "approved",
            "rejected",
            "in_progress",
            "completed",
            name="issue_status",
        ),
        default="pending_analysis",
        nullable=False,
    )
    fingerprint: Mapped[str] = mapped_column(String(64), nullable=True, index=True)
    submitter_ip_hash: Mapped[str] = mapped_column(String(64), nullable=True)
    attachment_urls: Mapped[list] = mapped_column(JSONB, nullable=True)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), server_default=func.now())
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), server_default=func.now(), onupdate=func.now()
    )

    # Relationships
    project: Mapped["Project"] = relationship(back_populates="issues")
    analysis: Mapped["IssueAnalysis"] = relationship(back_populates="issue", uselist=False)
    change_request: Mapped["ChangeRequest"] = relationship(back_populates="issue", uselist=False)
    admin_decisions: Mapped[list["AdminDecision"]] = relationship(back_populates="issue")


class IssueAnalysis(Base):
    __tablename__ = "issue_analyses"

    id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), primary_key=True, default=uuid.uuid4)
    issue_id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), ForeignKey("issue_entries.id"), nullable=False)
    authenticity_score: Mapped[float] = mapped_column(Float, default=1.0)
    feasibility_score: Mapped[float] = mapped_column(Float, nullable=True)
    priority_suggestion: Mapped[int] = mapped_column(Integer, nullable=True)
    category_suggestion: Mapped[str] = mapped_column(String(50), nullable=True)
    severity_suggestion: Mapped[str] = mapped_column(String(20), nullable=True)
    duplicate_of: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), ForeignKey("issue_entries.id"), nullable=True)
    affected_modules: Mapped[dict] = mapped_column(JSONB, nullable=True)
    analysis_summary: Mapped[str] = mapped_column(Text, nullable=True)
    raw_llm_output: Mapped[dict] = mapped_column(JSONB, nullable=True)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), server_default=func.now())

    # Relationships
    issue: Mapped["IssueEntry"] = relationship(back_populates="analysis")
