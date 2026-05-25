"""WebSocket connection manager for real-time state push to Review Portal."""
import json
from fastapi import WebSocket


class ConnectionManager:
    def __init__(self) -> None:
        # project_id -> list of active connections
        self._connections: dict[str, list[WebSocket]] = {}

    async def connect(self, websocket: WebSocket, project_id: str) -> None:
        await websocket.accept()
        self._connections.setdefault(project_id, []).append(websocket)

    def disconnect(self, websocket: WebSocket, project_id: str) -> None:
        connections = self._connections.get(project_id, [])
        if websocket in connections:
            connections.remove(websocket)

    async def broadcast(self, project_id: str, message: dict) -> None:
        connections = self._connections.get(project_id, [])
        dead = []
        for ws in connections:
            try:
                await ws.send_text(json.dumps(message))
            except Exception:
                dead.append(ws)
        for ws in dead:
            self.disconnect(ws, project_id)

    async def broadcast_all(self, message: dict) -> None:
        for project_id in list(self._connections.keys()):
            await self.broadcast(project_id, message)


ws_manager = ConnectionManager()
