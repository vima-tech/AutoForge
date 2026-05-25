from typing import Literal
from pydantic import BaseModel


class ReviewDecision(BaseModel):
    decision: Literal["approved", "rejected", "revision", "terminated", "rollback"]
    suggestions: str | None = None
    stage: Literal["review_1", "review_2", "bug_review"]
