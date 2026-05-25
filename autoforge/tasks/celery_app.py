from celery import Celery
from autoforge.config import settings

celery_app = Celery(
    "autoforge",
    broker=settings.celery_broker_url,
    backend=settings.celery_result_backend,
    include=[
        "autoforge.tasks.analysis",
        "autoforge.tasks.execution",
        "autoforge.tasks.testing",
    ],
)

celery_app.conf.update(
    task_serializer="json",
    result_serializer="json",
    accept_content=["json"],
    timezone="Asia/Shanghai",
    enable_utc=True,
    task_track_started=True,
    worker_prefetch_multiplier=1,  # one task at a time per worker for heavy Claude tasks
    beat_schedule={
        "daily-proactive-scan": {
            "task": "autoforge.tasks.testing.run_proactive_scan",
            "schedule": 86400,  # 24 hours
        },
    },
)
