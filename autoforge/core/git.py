"""
Git proxy layer with Layer 2 security audit.
All git operations for Claude Code pass through this module.
Unauthorized operations are intercepted and logged.
"""
import asyncio
import re
import uuid
from datetime import datetime
from autoforge.config import settings

# Operations that Claude Code must never perform
_FORBIDDEN_PATTERNS = [
    r"push\s+.*\bmain\b",
    r"push\s+.*\bmaster\b",
    r"push\s+--force",
    r"push\s+-f\b",
    r"branch\s+-[Dd]\s",
    r"symbolic-ref",
    r"update-ref",
    r"remote\s+set-url",
    r"config\s+--global",
]
_COMPILED_FORBIDDEN = [re.compile(p, re.IGNORECASE) for p in _FORBIDDEN_PATTERNS]


class GitSecurityViolation(Exception):
    pass


class GitProxy:
    def __init__(self, worktree_path: str, allowed_branch_prefix: str = "autoforge/"):
        self.worktree_path = worktree_path
        self.allowed_branch_prefix = allowed_branch_prefix
        self.audit_log: list[dict] = []

    def _audit(self, command: str, blocked: bool, reason: str = "") -> None:
        self.audit_log.append({
            "timestamp": datetime.utcnow().isoformat(),
            "command": command,
            "blocked": blocked,
            "reason": reason,
        })

    def _check_command(self, git_args: list[str]) -> None:
        command_str = " ".join(git_args)
        for pattern in _COMPILED_FORBIDDEN:
            if pattern.search(command_str):
                self._audit(command_str, blocked=True, reason=f"forbidden_pattern:{pattern.pattern}")
                raise GitSecurityViolation(
                    f"Git operation blocked by security policy: {command_str}"
                )
        self._audit(command_str, blocked=False)

    async def run(self, git_args: list[str], timeout: int = 60) -> tuple[int, str, str]:
        self._check_command(git_args)
        proc = await asyncio.create_subprocess_exec(
            "git",
            *git_args,
            cwd=self.worktree_path,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        try:
            stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=timeout)
        except asyncio.TimeoutError:
            proc.kill()
            raise
        return proc.returncode, stdout.decode(), stderr.decode()

    def get_audit_log(self) -> list[dict]:
        return list(self.audit_log)
