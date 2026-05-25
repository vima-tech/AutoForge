"""Celery tasks for preview environment lifecycle."""
import asyncio
from autoforge.tasks.celery_app import celery_app
from autoforge.services.preview_orchestrator import create_worktree_preview, destroy_worktree_preview


def _run_async(coro):
    loop = asyncio.new_event_loop()
    try:
        return loop.run_until_complete(coro)
    finally:
        loop.close()


@celery_app.task(name="autoforge.tasks.preview.build_preview", bind=True, max_retries=1)
def build_preview(self, session_id: str) -> dict:
    try:
        _run_async(create_worktree_preview(session_id))
        return {"status": "done", "session_id": session_id}
    except Exception as exc:
        raise self.retry(exc=exc, countdown=10)


@celery_app.task(name="autoforge.tasks.preview.destroy_preview", bind=True, max_retries=2)
def destroy_preview(self, env_id: str) -> dict:
    try:
        _run_async(destroy_worktree_preview(env_id))
        return {"status": "done", "env_id": env_id}
    except Exception as exc:
        raise self.retry(exc=exc, countdown=5)
