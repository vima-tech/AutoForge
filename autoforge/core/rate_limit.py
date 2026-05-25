"""
Sliding window rate limiter backed by Redis.
Protects all external input endpoints from abuse (design doc §6.1).
"""
import time
from redis.asyncio import Redis


class RateLimiter:
    def __init__(self, redis: Redis) -> None:
        self._redis = redis

    async def check(
        self,
        key: str,
        limit: int,
        window_seconds: int,
    ) -> tuple[bool, int]:
        """
        Sliding window rate check.
        Returns (allowed, remaining).
        """
        now = time.time()
        window_start = now - window_seconds

        pipe = self._redis.pipeline()
        # Remove expired entries
        pipe.zremrangebyscore(key, 0, window_start)
        # Count current window
        pipe.zcard(key)
        # Add current request
        pipe.zadd(key, {str(now): now})
        # Set TTL to clean up stale keys
        pipe.expire(key, window_seconds + 1)
        results = await pipe.execute()

        current_count = results[1]
        allowed = current_count < limit
        remaining = max(0, limit - current_count - 1)
        return allowed, remaining


# Default limits per source type
RATE_LIMITS = {
    "widget":  (30, 60),    # 30 requests per minute per IP
    "github":  (100, 60),   # 100/min (GitHub webhook bursts)
    "admin":   (200, 60),   # 200/min (admin is trusted)
    "webhook": (50, 60),    # 50/min for generic webhooks
    "default": (20, 60),
}
