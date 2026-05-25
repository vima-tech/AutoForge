import uuid
from fastapi import APIRouter, Depends, HTTPException, status
from pydantic import BaseModel
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession
from autoforge.database import get_db
from autoforge.models import Project
from autoforge.schemas import ProjectCreate, ProjectRead, ProjectUpdate
from autoforge.core.config_parser import parse_config, config_to_dict

router = APIRouter()


class ConfigUpload(BaseModel):
    yaml_content: str


@router.post("", response_model=ProjectRead, status_code=status.HTTP_201_CREATED)
async def create_project(body: ProjectCreate, db: AsyncSession = Depends(get_db)) -> Project:
    existing = await db.scalar(select(Project).where(Project.slug == body.slug))
    if existing:
        raise HTTPException(status_code=409, detail=f"Slug '{body.slug}' already exists")
    project = Project(**body.model_dump())
    db.add(project)
    await db.commit()
    await db.refresh(project)
    return project


@router.get("", response_model=list[ProjectRead])
async def list_projects(db: AsyncSession = Depends(get_db)) -> list[Project]:
    result = await db.scalars(select(Project).order_by(Project.created_at.desc()))
    return list(result.all())


@router.get("/{project_id}", response_model=ProjectRead)
async def get_project(project_id: uuid.UUID, db: AsyncSession = Depends(get_db)) -> Project:
    project = await db.get(Project, project_id)
    if not project:
        raise HTTPException(status_code=404, detail="Project not found")
    return project


@router.patch("/{project_id}", response_model=ProjectRead)
async def update_project(
    project_id: uuid.UUID, body: ProjectUpdate, db: AsyncSession = Depends(get_db)
) -> Project:
    project = await db.get(Project, project_id)
    if not project:
        raise HTTPException(status_code=404, detail="Project not found")
    for field, value in body.model_dump(exclude_none=True).items():
        setattr(project, field, value)
    await db.commit()
    await db.refresh(project)
    return project


@router.post("/{project_id}/config")
async def upload_config(
    project_id: uuid.UUID,
    body: ConfigUpload,
    db: AsyncSession = Depends(get_db),
) -> dict:
    """Upload and validate autoforge.yaml for a project."""
    project = await db.get(Project, project_id)
    if not project:
        raise HTTPException(status_code=404, detail="Project not found")

    try:
        config = parse_config(body.yaml_content)
    except Exception as e:
        raise HTTPException(status_code=422, detail=f"Invalid autoforge.yaml: {e}")

    project.config_snapshot = config_to_dict(config)
    await db.commit()
    return {"status": "ok", "config": project.config_snapshot}


@router.get("/{project_id}/config")
async def get_config(project_id: uuid.UUID, db: AsyncSession = Depends(get_db)) -> dict:
    project = await db.get(Project, project_id)
    if not project:
        raise HTTPException(status_code=404, detail="Project not found")
    return project.config_snapshot or {}


@router.post("/{project_id}/config/validate")
async def validate_config(body: ConfigUpload) -> dict:
    """Validate autoforge.yaml without saving."""
    try:
        config = parse_config(body.yaml_content)
        return {"valid": True, "config": config_to_dict(config)}
    except Exception as e:
        return {"valid": False, "error": str(e)}
