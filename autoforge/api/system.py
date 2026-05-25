from fastapi import APIRouter
from autoforge.core.concurrency import concurrency_manager

router = APIRouter()


@router.get("/status")
async def system_status() -> dict:
    """System health and concurrency status for Review Portal dashboard."""
    return {
        "status": "ok",
        "concurrency": concurrency_manager.status_snapshot(),
    }


@router.get("/health")
async def health() -> dict:
    return {"status": "ok"}
