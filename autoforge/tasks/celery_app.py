from celery import Celery
from celery.schedules import crontab
from autoforge.config import settings

celery_app = Celery(
    "autoforge",
    broker=settings.celery_broker_url,
    backend=settings.celery_result_backend,
    include=[
        "autoforge.tasks.analysis",
        "autoforge.tasks.execution",
        "autoforge.tasks.testing",
        "autoforge.tasks.preview",
        "autoforge.tasks.merge",
    ],
)

celery_app.conf.update(
    task_serializer="json",
    result_serializer="json",
    accept_content=["json"],
    timezone="Asia/Shanghai",
    enable_utc=True,
    task_track_started=True,
    worker_prefetch_multiplier=1,  # one heavy task at a time per worker
    task_acks_late=True,
    beat_schedule={
        # M8: Daily proactive scan at 2:00 AM CST
        "daily-proactive-scan": {
            "task": "autoforge.tasks.testing.run_proactive_scan",
            "schedule": crontab(hour=2, minute=0),
        },
    },
)
