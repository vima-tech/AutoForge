from pydantic_settings import BaseSettings, SettingsConfigDict
from pydantic import Field


class Settings(BaseSettings):
    model_config = SettingsConfigDict(env_file=".env", env_file_encoding="utf-8", extra="ignore")

    # Application
    app_name: str = "AutoForge"
    app_version: str = "0.1.0"
    debug: bool = False
    secret_key: str = Field(..., description="JWT signing secret")

    # Database
    database_url: str = Field(..., description="PostgreSQL connection URL")

    # Redis / Celery
    redis_url: str = "redis://localhost:6379/0"
    celery_broker_url: str = "redis://localhost:6379/1"
    celery_result_backend: str = "redis://localhost:6379/2"

    # Anthropic
    anthropic_api_key: str = Field(..., description="Anthropic API key for agents")

    # Preview system
    preview_base_url: str = "http://localhost:9000"
    preview_db_host: str = "localhost"
    preview_db_port: int = 5433
    preview_db_user: str = "preview"
    preview_db_password: str = Field(..., description="Preview DB password")

    # Concurrency
    max_concurrent_slots: int = 5
    backpressure_pause_threshold: int = 20

    # Docker
    docker_host: str = "unix:///var/run/docker.sock"
    preview_container_cpu_limit: float = 1.0
    preview_container_mem_limit: str = "512m"

    # Notifications
    smtp_host: str = ""
    smtp_port: int = 587
    smtp_user: str = ""
    smtp_password: str = ""
    slack_webhook_url: str = ""

    # Security
    access_token_expire_minutes: int = 60 * 24  # 24 hours
    layer1_sanitize_model: str = "claude-haiku-4-5-20251001"
    github_webhook_secret: str = ""   # HMAC secret registered in GitHub webhook settings
    admin_api_key: str = ""           # Shared secret for Review Portal admin actions


settings = Settings()
