"""
Backpressure-based concurrent slot management.
Implements the three-stage flow control from design doc §7.1.
"""
import asyncio
from dataclasses import dataclass, field
from enum import Enum
from autoforge.config import settings


class SystemStage(str, Enum):
    NORMAL = "normal"         # < max_slots pending reviews
    THROTTLED = "throttled"   # all slots occupied by pending reviews
    PAUSED = "paused"         # pending_review count >= pause_threshold


@dataclass
class ConcurrencyManager:
    max_slots: int = field(default_factory=lambda: settings.max_concurrent_slots)
    pause_threshold: int = field(default_factory=lambda: settings.backpressure_pause_threshold)

    _active_slots: int = field(default=0, init=False)
    _pending_review_count: int = field(default=0, init=False)
    _waiting_queue: list = field(default_factory=list, init=False)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, init=False)

    def get_stage(self) -> SystemStage:
        if self._pending_review_count >= self.pause_threshold:
            return SystemStage.PAUSED
        if self._active_slots >= self.max_slots:
            return SystemStage.THROTTLED
        return SystemStage.NORMAL

    @property
    def effective_concurrency(self) -> int:
        stage = self.get_stage()
        if stage == SystemStage.PAUSED:
            return 0
        if stage == SystemStage.THROTTLED:
            return 1
        return self.max_slots

    async def acquire_slot(self, cr_id: str) -> bool:
        async with self._lock:
            stage = self.get_stage()
            if stage == SystemStage.PAUSED:
                return False
            current_running = self._active_slots - self._pending_review_count
            limit = self.effective_concurrency
            if current_running < limit:
                self._active_slots += 1
                return True
            return False

    async def mark_pending_review(self) -> None:
        async with self._lock:
            self._pending_review_count += 1

    async def release_slot(self) -> None:
        async with self._lock:
            self._active_slots = max(0, self._active_slots - 1)
            self._pending_review_count = max(0, self._pending_review_count - 1)

    def status_snapshot(self) -> dict:
        return {
            "stage": self.get_stage().value,
            "active_slots": self._active_slots,
            "pending_review_count": self._pending_review_count,
            "max_slots": self.max_slots,
            "pause_threshold": self.pause_threshold,
            "effective_concurrency": self.effective_concurrency,
        }


# Singleton instance
concurrency_manager = ConcurrencyManager()
