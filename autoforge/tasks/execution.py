"""
Claude Code Agent execution task.
Manages worktree lifecycle and invokes Claude Code.
Implemented in M4. Stub here for M1 skeleton.
"""
from autoforge.tasks.celery_app import celery_app


@celery_app.task(name="autoforge.tasks.execution.run_claude_code", bind=True, max_retries=0)
def run_claude_code(self, change_request_id: str) -> dict:
    return {"status": "stub", "change_request_id": change_request_id}
