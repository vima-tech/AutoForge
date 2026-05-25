"""
WebSocket connection manager with Redis Pub/Sub for cross-process broadcasting.
Supports multiple uvicorn workers and Celery tasks broadcasting events.
Design doc §8.2.
"""
import asyncio
import json
import logging
from fastapi import WebSocket

logger = logging.getLogger(__name__)

CHANNEL_PREFIX = "ws_events:"


class ConnectionManager:
    def __init__(self) -> None:
        # project_id -> list of active WebSocket connections (in-process)
        self._connections: dict[str, list[WebSocket]] = {}
        self._pubsub_task: asyncio.Task | None = None

    async def connect(self, websocket: WebSocket, project_id: str) -> None:
        await websocket.accept()
        self._connections.setdefault(project_id, []).append(websocket)

    def disconnect(self, websocket: WebSocket, project_id: str) -> None:
        connections = self._connections.get(project_id, [])
        if websocket in connections:
            connections.remove(websocket)

    async def _send_to_local_connections(self, project_id: str, message: dict) -> None:
        """Forward a message to all WebSocket connections in this process."""
        connections = self._connections.get(project_id, [])
        dead = []
        for ws in connections:
            try:
                await ws.send_text(json.dumps(message))
            except Exception:
                dead.append(ws)
        for ws in dead:
            self.disconnect(ws, project_id)

    async def broadcast(self, project_id: str, message: dict) -> None:
        """
        Broadcast to all connected clients via Redis Pub/Sub.
        Works across multiple processes and workers.
        """
        try:
            from autoforge.core.redis_client import get_redis
            redis = get_redis()
            channel = f"{CHANNEL_PREFIX}{project_id}"
            await redis.publish(channel, json.dumps(message))
        except Exception as e:
            # Fallback to local-only broadcast if Redis is unavailable
            logger.warning("Redis broadcast failed, falling back to local: %s", e)
            await self._send_to_local_connections(project_id, message)

    async def broadcast_all(self, message: dict) -> None:
        for project_id in list(self._connections.keys()):
            await self.broadcast(project_id, message)

    async def start_pubsub_listener(self) -> None:
        """Start background task that listens to Redis Pub/Sub and forwards to local WS."""
        if self._pubsub_task and not self._pubsub_task.done():
            return
        self._pubsub_task = asyncio.create_task(self._pubsub_loop())

    async def _pubsub_loop(self) -> None:
        """Long-running loop that subscribes to all ws_events:* channels."""
        while True:
            try:
                from autoforge.core.redis_client import get_redis
                redis = get_redis()
                pubsub = redis.pubsub()
                await pubsub.psubscribe(f"{CHANNEL_PREFIX}*")

                async for message in pubsub.listen():
                    if message["type"] != "pmessage":
                        continue
                    channel: str = message["channel"]
                    project_id = channel.removeprefix(CHANNEL_PREFIX)
                    try:
                        data = json.loads(message["data"])
                        await self._send_to_local_connections(project_id, data)
                    except Exception:
                        pass

            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.warning("Pub/Sub listener error, reconnecting in 3s: %s", e)
                await asyncio.sleep(3)


ws_manager = ConnectionManager()
