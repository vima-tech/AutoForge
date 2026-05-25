import pytest
from autoforge.core.git import GitProxy, GitSecurityViolation


def proxy():
    return GitProxy(worktree_path="/tmp/test", allowed_branch_prefix="autoforge/")


def test_allows_safe_commit():
    p = proxy()
    p._check_command(["commit", "-m", "fix: add feature"])
    assert len(p.audit_log) == 1
    assert not p.audit_log[0]["blocked"]


def test_blocks_push_to_main():
    p = proxy()
    with pytest.raises(GitSecurityViolation):
        p._check_command(["push", "origin", "main"])
    assert p.audit_log[0]["blocked"]


def test_blocks_force_push():
    p = proxy()
    with pytest.raises(GitSecurityViolation):
        p._check_command(["push", "--force", "origin", "autoforge/cr-1"])
    assert p.audit_log[0]["blocked"]


def test_blocks_branch_delete():
    p = proxy()
    with pytest.raises(GitSecurityViolation):
        p._check_command(["branch", "-D", "dev"])
    assert p.audit_log[0]["blocked"]


def test_allows_worktree_branch_push():
    p = proxy()
    p._check_command(["push", "origin", "autoforge/cr-042"])
    assert not p.audit_log[0]["blocked"]
