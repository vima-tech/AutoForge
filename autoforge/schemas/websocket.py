from typing import Any, Literal
from pydantic import BaseModel


class WSMessage(BaseModel):
    type: Literal[
        "worktree_update",
        "preview_ready",
        "preview_terminated",
        "test_result",
        "issue_created",
        "review_needed",
        "system_status",
    ]
    payload: dict[str, Any]
