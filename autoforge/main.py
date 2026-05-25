from contextlib import asynccontextmanager
from fastapi import FastAPI, WebSocket, WebSocketDisconnect
from fastapi.middleware.cors import CORSMiddleware
from autoforge.api import router
from autoforge.config import settings
from autoforge.websocket.manager import ws_manager


@asynccontextmanager
async def lifespan(app: FastAPI):
    yield


app = FastAPI(
    title="AutoForge",
    description="Human-Lite-in-the-Loop autonomous software factory",
    version=settings.app_version,
    lifespan=lifespan,
)

app.add_middleware(
    CORSMiddleware,
    allow_origins=["http://localhost:3000"],  # Review Portal dev server
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

app.include_router(router, prefix="/api/v1")


@app.websocket("/ws/{project_id}")
async def websocket_endpoint(websocket: WebSocket, project_id: str) -> None:
    await ws_manager.connect(websocket, project_id)
    try:
        while True:
            await websocket.receive_text()  # keep connection alive
    except WebSocketDisconnect:
        ws_manager.disconnect(websocket, project_id)
