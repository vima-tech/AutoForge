from contextlib import asynccontextmanager
from fastapi import FastAPI, WebSocket, WebSocketDisconnect
from fastapi.middleware.cors import CORSMiddleware
from fastapi.staticfiles import StaticFiles
from pathlib import Path
from autoforge.api import router
from autoforge.config import settings
from autoforge.websocket.manager import ws_manager


@asynccontextmanager
async def lifespan(app: FastAPI):
    # Start Redis Pub/Sub listener for cross-process WebSocket broadcasting
    await ws_manager.start_pubsub_listener()

    yield

    # Cleanup
    if ws_manager._pubsub_task:
        ws_manager._pubsub_task.cancel()


app = FastAPI(
    title="AutoForge",
    description="Human-Lite-in-the-Loop autonomous software factory",
    version=settings.app_version,
    lifespan=lifespan,
)

app.add_middleware(
    CORSMiddleware,
    allow_origins=["http://localhost:3000", "*"],  # * for widget cross-origin
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

app.include_router(router, prefix="/api/v1")

# Serve the embeddable widget JS
widget_dir = Path(__file__).parent.parent / "widget"
if widget_dir.exists():
    app.mount("/widget", StaticFiles(directory=str(widget_dir)), name="widget")


@app.websocket("/ws/{project_id}")
async def websocket_endpoint(websocket: WebSocket, project_id: str) -> None:
    await ws_manager.connect(websocket, project_id)
    try:
        while True:
            await websocket.receive_text()
    except WebSocketDisconnect:
        ws_manager.disconnect(websocket, project_id)
