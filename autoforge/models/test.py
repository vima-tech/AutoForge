import uuid
from datetime import datetime
from sqlalchemy import String, Text, Integer, Enum, DateTime, ForeignKey, func
from sqlalchemy.dialects.postgresql import UUID, JSONB
from sqlalchemy.orm import Mapped, mapped_column, relationship
from autoforge.database import Base


class TestSession(Base):
    __tablename__ = "test_sessions"

    id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), primary_key=True, default=uuid.uuid4)
    project_id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), ForeignKey("projects.id"), nullable=False)
    session_type: Mapped[str] = mapped_column(
        Enum("reactive", "proactive", name="test_session_type"),
        nullable=False,
    )
    change_request_id: Mapped[uuid.UUID] = mapped_column(
        UUID(as_uuid=True), ForeignKey("change_requests.id"), nullable=True
    )
    trigger: Mapped[str] = mapped_column(
        Enum("merge", "scheduled", "manual", name="test_trigger"),
        nullable=False,
    )
    status: Mapped[str] = mapped_column(String(50), default="pending", nullable=False)
    # Status values: pending | running | passed | failed | error
    summary: Mapped[str] = mapped_column(Text, nullable=True)
    results_json: Mapped[dict] = mapped_column(JSONB, nullable=True)
    issues_created: Mapped[list] = mapped_column(JSONB, nullable=True)
    started_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=True)
    completed_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=True)

    # Relationships
    project: Mapped["Project"] = relationship(back_populates="test_sessions")
    findings: Mapped[list["ScanFinding"]] = relationship(back_populates="test_session")


class ScanFinding(Base):
    __tablename__ = "scan_findings"

    id: Mapped[uuid.UUID] = mapped_column(UUID(as_uuid=True), primary_key=True, default=uuid.uuid4)
    test_session_id: Mapped[uuid.UUID] = mapped_column(
        UUID(as_uuid=True), ForeignKey("test_sessions.id"), nullable=False
    )
    check_type: Mapped[str] = mapped_column(String(100), nullable=False)
    severity: Mapped[str] = mapped_column(
        Enum("critical", "high", "medium", "low", name="finding_severity"),
        nullable=False,
    )
    title: Mapped[str] = mapped_column(String(500), nullable=False)
    description: Mapped[str] = mapped_column(Text, nullable=True)
    file_path: Mapped[str] = mapped_column(String(500), nullable=True)
    line_number: Mapped[int] = mapped_column(Integer, nullable=True)
    fingerprint: Mapped[str] = mapped_column(String(64), nullable=True, index=True)
    issue_entry_id: Mapped[uuid.UUID] = mapped_column(
        UUID(as_uuid=True), ForeignKey("issue_entries.id"), nullable=True
    )
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), server_default=func.now())

    # Relationships
    test_session: Mapped["TestSession"] = relationship(back_populates="findings")
