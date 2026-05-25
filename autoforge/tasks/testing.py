"""
Test Agent tasks — reactive (post-merge) and proactive (scheduled).
Implemented in M7/M8. Stub here for M1 skeleton.
"""
from autoforge.tasks.celery_app import celery_app


@celery_app.task(name="autoforge.tasks.testing.run_reactive_test", bind=True, max_retries=1)
def run_reactive_test(self, change_request_id: str) -> dict:
    return {"status": "stub", "change_request_id": change_request_id}


@celery_app.task(name="autoforge.tasks.testing.run_proactive_scan", bind=True)
def run_proactive_scan(self) -> dict:
    return {"status": "stub"}
