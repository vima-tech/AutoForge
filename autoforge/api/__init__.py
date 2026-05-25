from fastapi import APIRouter
from autoforge.api import projects, issues, change_requests, reviews, preview, system

router = APIRouter()
router.include_router(projects.router, prefix="/projects", tags=["projects"])
router.include_router(issues.router, prefix="/issues", tags=["issues"])
router.include_router(change_requests.router, prefix="/change-requests", tags=["change-requests"])
router.include_router(reviews.router, prefix="/reviews", tags=["reviews"])
router.include_router(preview.router, prefix="/preview", tags=["preview"])
router.include_router(system.router, prefix="/system", tags=["system"])
