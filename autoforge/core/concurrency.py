"""
Redis-backed concurrency slot manager (replaces in-memory version from M1).
Supports multi-process Celery workers and multiple uvicorn instances.
Design doc §7.1, §11.1.
"""
from enum import Enum
from redis.asyncio import Redis
from autoforge.config import settings


class SystemStage(str, Enum):
    NORMAL = "normal"
    THROTTLED = "throttled"
    PAUSED = "paused"


_ACTIVE_KEY = "autoforge:slots:active"
_PENDING_REVIEW_KEY = "autoforge:slots:pending_review"


class ConcurrencyManager:
    def __init__(
        self,
        max_slots: int | None = None,
        pause_threshold: int | None = None,
    ) -> None:
        self.max_slots = max_slots or settings.max_concurrent_slots
        self.pause_threshold = pause_threshold or settings.backpressure_pause_threshold

    def _redis(self) -> Redis:
        from autoforge.core.redis_client import get_redis
        return get_redis()

    async def _counts(self) -> tuple[int, int]:
        r = self._redis()
        active = int(await r.get(_ACTIVE_KEY) or 0)
        pending = int(await r.get(_PENDING_REVIEW_KEY) or 0)
        return active, pending

    def get_stage_from(self, active: int, pending: int) -> SystemStage:
        if pending >= self.pause_threshold:
            return SystemStage.PAUSED
        if active >= self.max_slots:
            return SystemStage.THROTTLED
        return SystemStage.NORMAL

    async def get_stage(self) -> SystemStage:
        active, pending = await self._counts()
        return self.get_stage_from(active, pending)

    async def effective_concurrency(self) -> int:
        stage = await self.get_stage()
        if stage == SystemStage.PAUSED:
            return 0
        if stage == SystemStage.THROTTLED:
            return 1
        return self.max_slots

    async def acquire_slot(self, cr_id: str) -> bool:
        r = self._redis()
        active, pending = await self._counts()
        stage = self.get_stage_from(active, pending)

        if stage == SystemStage.PAUSED:
            return False

        running = active - pending
        limit = 1 if stage == SystemStage.THROTTLED else self.max_slots
        if running >= limit:
            return False

        await r.incr(_ACTIVE_KEY)
        return True

    async def mark_pending_review(self) -> None:
        await self._redis().incr(_PENDING_REVIEW_KEY)

    async def release_slot(self) -> None:
        r = self._redis()
        await r.decr(_ACTIVE_KEY)
        current_pending = int(await r.get(_PENDING_REVIEW_KEY) or 0)
        if current_pending > 0:
            await r.decr(_PENDING_REVIEW_KEY)

    async def status_snapshot(self) -> dict:
        active, pending = await self._counts()
        stage = self.get_stage_from(active, pending)
        return {
            "stage": stage.value,
            "active_slots": active,
            "pending_review_count": pending,
            "max_slots": self.max_slots,
            "pause_threshold": self.pause_threshold,
            "effective_concurrency": (
                0 if stage == SystemStage.PAUSED
                else 1 if stage == SystemStage.THROTTLED
                else self.max_slots
            ),
        }

    async def update_config(self, max_slots: int, pause_threshold: int) -> None:
        """Hot-update limits without restart. Persisted only in-memory for this process."""
        self.max_slots = max_slots
        self.pause_threshold = pause_threshold

    async def reset(self) -> None:
        """Reset all counters — use only in tests."""
        r = self._redis()
        await r.delete(_ACTIVE_KEY, _PENDING_REVIEW_KEY)


concurrency_manager = ConcurrencyManager()
