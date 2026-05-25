import uuid
from fastapi import APIRouter, Depends, HTTPException, status
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession
from autoforge.database import get_db
from autoforge.models import Project
from autoforge.schemas import ProjectCreate, ProjectRead, ProjectUpdate

router = APIRouter()


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
