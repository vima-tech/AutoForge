"""
Preview Orchestrator — manages the full lifecycle of preview environments.
Design doc §5.1, §5.2.

Lifecycle per worktree:
  create_worktree_preview → seed DB → mask → build container → health check →
  update nginx → notify ready → [admin review] → destroy on decision
"""
import asyncio
import logging
import re
import subprocess
import time
import uuid as uuid_mod
from datetime import datetime, timezone
from pathlib import Path

import docker
from docker.errors import DockerException

from autoforge.config import settings
from autoforge.database import AsyncSessionLocal
from autoforge.models import PreviewEnvironment, WorktreeSession, Project, ChangeRequest
from autoforge.services.masking import apply_masking
from autoforge.websocket.manager import ws_manager

logger = logging.getLogger(__name__)

NGINX_CONF_PATH = Path(settings.__dict__.get("preview_nginx_conf_path", "/etc/nginx/conf.d/autoforge-previews.conf"))
PREVIEW_PORT_START = int(settings.__dict__.get("preview_port_start", 9100))


def _get_docker_client() -> docker.DockerClient:
    return docker.DockerClient(base_url=settings.docker_host)


def _preview_db_url(db_name: str) -> str:
    return (
        f"postgresql://{settings.preview_db_user}:{settings.preview_db_password}"
        f"@{settings.preview_db_host}:{settings.preview_db_port}/{db_name}"
    )


async def _create_preview_db(db_name: str, seed_command: str | None, sensitive_fields: list) -> str:
    """Create isolated preview database, run seed, apply masking. Returns db_url."""
    import psycopg2

    admin_url = _preview_db_url("postgres")

    # CREATE DATABASE must run outside a transaction
    with psycopg2.connect(admin_url) as conn:
        conn.autocommit = True
        with conn.cursor() as cur:
            # Sanitize db_name to avoid SQL injection
            safe_name = re.sub(r"[^a-z0-9_]", "_", db_name.lower())[:60]
            cur.execute(f'CREATE DATABASE "{safe_name}"')

    db_url = _preview_db_url(safe_name)

    # Run seed command if provided
    if seed_command:
        result = subprocess.run(
            seed_command,
            shell=True,
            capture_output=True,
            text=True,
            timeout=120,
            env={"DATABASE_URL": db_url, "PATH": "/usr/bin:/bin:/usr/local/bin"},
        )
        if result.returncode != 0:
            logger.warning("Seed command failed: %s", result.stderr[:500])

    # Apply field-level masking
    if sensitive_fields:
        apply_masking(db_url=db_url, sensitive_fields=sensitive_fields)

    return db_url


def _allocate_port(env_id: str) -> int:
    """Allocate a deterministic port based on environment ID."""
    return PREVIEW_PORT_START + (hash(env_id) % 900)


def _update_nginx_conf(env_id: str, port: int, project_slug: str) -> None:
    """Append nginx location block for this preview environment."""
    block = (
        f"\n# preview {env_id}\n"
        f"location /{project_slug}/cr-{env_id}/ {{\n"
        f"    proxy_pass http://127.0.0.1:{port}/;\n"
        f"    proxy_set_header Host $host;\n"
        f"    proxy_set_header X-Real-IP $remote_addr;\n"
        f"}}\n"
    )
    NGINX_CONF_PATH.parent.mkdir(parents=True, exist_ok=True)
    with open(NGINX_CONF_PATH, "a") as f:
        f.write(block)

    # Reload nginx
    subprocess.run(["nginx", "-s", "reload"], capture_output=True)


def _remove_nginx_location(env_id: str) -> None:
    """Remove nginx location block for this environment."""
    if not NGINX_CONF_PATH.exists():
        return
    content = NGINX_CONF_PATH.read_text()
    # Remove the block between comment markers
    pattern = rf"\n# preview {re.escape(env_id)}\n.*?\}}\n"
    cleaned = re.sub(pattern, "", content, flags=re.DOTALL)
    NGINX_CONF_PATH.write_text(cleaned)
    subprocess.run(["nginx", "-s", "reload"], capture_output=True)


async def create_worktree_preview(session_id: str) -> None:
    """
    Full preview creation pipeline for a completed worktree session.
    Called from Celery task after Claude Code finishes.
    """
    async with AsyncSessionLocal() as db:
        session = await db.get(WorktreeSession, uuid_mod.UUID(session_id))
        if not session:
            logger.error("WorktreeSession %s not found", session_id)
            return

        cr = await db.get(ChangeRequest, session.change_request_id)
        project = await db.get(Project, cr.project_id) if cr else None

        if not project:
            logger.error("Project not found for session %s", session_id)
            return

        project_config = project.config_snapshot or {}
        preview_config = project_config.get("preview", {})
        seed_command = (preview_config.get("seed") or {}).get("command")
        sensitive_fields = preview_config.get("sensitive_fields", [])
        start_command = (preview_config.get("start") or {}).get("command", "")
        container_port = (preview_config.get("start") or {}).get("port", 8000)
        health_path = (preview_config.get("start") or {}).get("health_check", "GET /health")
        ready_timeout = (preview_config.get("start") or {}).get("ready_timeout", 30)
        image = project_config.get("docker_image", f"autoforge/{project.slug}:latest")

        # Create PreviewEnvironment record
        env = PreviewEnvironment(
            project_id=project.id,
            env_type="worktree",
            worktree_session_id=session.id,
            status="building",
        )
        db.add(env)
        await db.commit()
        await db.refresh(env)

        env_id = str(env.id)
        db_name = f"preview_{project.slug}_{str(cr.id)[:8]}_{int(time.time())}"

        try:
            # Create and seed preview database
            db_url = await _create_preview_db(db_name, seed_command, sensitive_fields)
            env.db_snapshot_name = db_name
            env.data_masked_at = datetime.now(timezone.utc)
            await db.commit()

            # Launch Docker container
            host_port = _allocate_port(env_id)
            docker_client = _get_docker_client()

            env_vars_config = preview_config.get("env", {})
            container_env = {
                "DATABASE_URL": db_url,
                "SECRET_KEY": settings.secret_key,
            }
            for k, v in env_vars_config.items():
                if not v.startswith("${"):
                    container_env[k] = v

            container = docker_client.containers.run(
                image=image,
                command=start_command,
                environment=container_env,
                ports={f"{container_port}/tcp": host_port},
                mem_limit=settings.preview_container_mem_limit,
                nano_cpus=int(settings.preview_container_cpu_limit * 1e9),
                detach=True,
                remove=False,
                labels={"autoforge.env_id": env_id, "autoforge.project": project.slug},
            )
            env.container_id = container.id
            await db.commit()

            # Health check loop
            health_method, health_path_part = health_path.split(" ", 1) if " " in health_path else ("GET", health_path)
            health_url = f"http://127.0.0.1:{host_port}{health_path_part}"
            deadline = time.monotonic() + ready_timeout

            import httpx
            async with httpx.AsyncClient() as client:
                while time.monotonic() < deadline:
                    try:
                        resp = await client.get(health_url, timeout=2.0)
                        if resp.status_code < 400:
                            break
                    except Exception:
                        pass
                    await asyncio.sleep(2)
                else:
                    raise TimeoutError(f"Container health check timed out after {ready_timeout}s")

            # Update nginx config
            _update_nginx_conf(env_id, host_port, project.slug)
            preview_url = f"{settings.preview_base_url}/{project.slug}/cr-{env_id}"

            env.preview_url = preview_url
            env.status = "ready"
            env.ready_at = datetime.now(timezone.utc)
            await db.commit()

            # Notify manager
            await ws_manager.broadcast(
                str(project.id),
                {
                    "type": "preview_ready",
                    "payload": {
                        "cr_id": str(cr.id),
                        "env_id": env_id,
                        "preview_url": preview_url,
                        "session_id": session_id,
                    },
                },
            )
            logger.info("Preview ready for session %s at %s", session_id, preview_url)

        except Exception as exc:
            logger.error("Preview build failed for session %s: %s", session_id, exc)
            env.status = "failed"
            await db.commit()
            await ws_manager.broadcast(
                str(project.id),
                {"type": "worktree_update", "payload": {"cr_id": str(cr.id), "preview_status": "failed"}},
            )


async def destroy_worktree_preview(env_id: str) -> None:
    """Stop container, drop preview DB, remove nginx config."""
    async with AsyncSessionLocal() as db:
        env = await db.get(PreviewEnvironment, uuid_mod.UUID(env_id))
        if not env:
            return

        # Stop and remove container
        if env.container_id:
            try:
                docker_client = _get_docker_client()
                container = docker_client.containers.get(env.container_id)
                container.stop(timeout=5)
                container.remove()
            except DockerException as e:
                logger.warning("Container cleanup failed: %s", e)

        # Drop preview database
        if env.db_snapshot_name:
            try:
                import psycopg2
                with psycopg2.connect(_preview_db_url("postgres")) as conn:
                    conn.autocommit = True
                    with conn.cursor() as cur:
                        safe_name = re.sub(r"[^a-z0-9_]", "_", env.db_snapshot_name.lower())[:60]
                        cur.execute(f'DROP DATABASE IF EXISTS "{safe_name}"')
            except Exception as e:
                logger.warning("Preview DB cleanup failed: %s", e)

        # Remove nginx location
        _remove_nginx_location(env_id)

        env.status = "terminated"
        env.terminated_at = datetime.now(timezone.utc)
        await db.commit()
        logger.info("Preview %s destroyed", env_id)
