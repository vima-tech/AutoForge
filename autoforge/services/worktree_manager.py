"""
Worktree lifecycle management.
Creates, monitors, and cleans up git worktrees for Claude Code sessions.
Design doc §11.1.
"""
import asyncio
import os
import shutil
import uuid as uuid_mod
from pathlib import Path
from autoforge.config import settings
from autoforge.core.git import GitProxy, GitSecurityViolation


WORKTREES_BASE = Path(os.environ.get("WORKTREES_BASE", "/tmp/autoforge-worktrees"))


class WorktreeError(Exception):
    pass


async def create_worktree(
    *,
    repo_path: str,
    branch_prefix: str,
    cr_id: str,
    base_branch: str = "dev",
) -> tuple[str, str]:
    """
    Create a git worktree for a change request.
    Returns (worktree_path, branch_name).
    """
    WORKTREES_BASE.mkdir(parents=True, exist_ok=True)
    branch_name = f"{branch_prefix}{cr_id}"
    worktree_path = str(WORKTREES_BASE / f"cr-{cr_id}")

    # Remove stale worktree if exists
    if Path(worktree_path).exists():
        shutil.rmtree(worktree_path)

    proxy = GitProxy(worktree_path=repo_path, allowed_branch_prefix=branch_prefix)

    # Create worktree on a new branch from base_branch
    rc, stdout, stderr = await proxy.run(
        ["worktree", "add", "-b", branch_name, worktree_path, base_branch],
        timeout=30,
    )
    if rc != 0:
        raise WorktreeError(f"Failed to create worktree: {stderr}")

    return worktree_path, branch_name


async def remove_worktree(*, repo_path: str, worktree_path: str, branch_name: str) -> None:
    """Remove worktree and delete its branch."""
    proxy = GitProxy(worktree_path=repo_path)

    await proxy.run(["worktree", "remove", "--force", worktree_path], timeout=30)

    # Delete the feature branch
    rc, _, err = await proxy.run(["branch", "-d", branch_name], timeout=15)
    if rc != 0:
        # Force delete if not fully merged
        await proxy.run(["branch", "-D", branch_name], timeout=15)


async def merge_worktree_to_dev(
    *,
    repo_path: str,
    branch_name: str,
    dev_branch: str = "dev",
) -> bool:
    """
    Merge worktree branch into dev (Layer 3: triggered only by explicit admin approval).
    Returns True on success.
    """
    proxy = GitProxy(worktree_path=repo_path)

    # Switch to dev
    rc, _, err = await proxy.run(["checkout", dev_branch], timeout=15)
    if rc != 0:
        raise WorktreeError(f"Cannot checkout {dev_branch}: {err}")

    # Merge with no-fast-forward for traceability
    rc, stdout, stderr = await proxy.run(
        ["merge", "--no-ff", branch_name, "-m", f"merge: {branch_name} via AutoForge approval"],
        timeout=60,
    )
    if rc != 0:
        # Abort merge on conflict
        await proxy.run(["merge", "--abort"], timeout=15)
        raise WorktreeError(f"Merge conflict in {branch_name}: {stderr}")

    return True
