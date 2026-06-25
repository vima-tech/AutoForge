#!/usr/bin/env python3
"""PreToolUse 硬拦截钩子(BRANCHING.md 的强制层)。
拦截违反分支规范的 Bash / Write / Edit 操作:
  - 在主仓 <dev> 只读镜像上编辑文件
  - 在 <dev> 分支上 git commit
  - 裸 git pull(非 --ff-only / --rebase)
  - git merge <dev>(应改 rebase)
  - force push / push 到 main|master / 直接 push 到 <dev>(应走 land)
  - 在主仓把 <dev> 切走(破坏 AutoForge 的 dev_is_live 安全路径)
放行 scripts/branch/ 下的 land/wt-new/wt-clean(它们内部自管)。
逃生阀:环境变量 AUTOFORGE_GUARD_OFF=1 时全部放行(用于维护本机制/引导)。
deny 输出 hookSpecificOutput.permissionDecision=deny。"""
import json, os, re, subprocess, sys

DEV = os.environ.get("AUTOFORGE_DEV_BRANCH", "dev")


def git(cwd, *args):
    try:
        r = subprocess.run(["git", "-C", cwd, *args], capture_output=True,
                           text=True, timeout=10)
        return r.returncode, r.stdout.strip(), r.stderr.strip()
    except Exception:
        return 1, "", ""


def deny(reason):
    print(json.dumps({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": reason,
    }}, ensure_ascii=False))
    sys.exit(0)


def repo_info(d):
    """返回 (is_inside, is_main_worktree, current_branch) for dir d。"""
    code, _, _ = git(d, "rev-parse", "--is-inside-work-tree")
    if code != 0:
        return False, False, ""
    _, common, _ = git(d, "rev-parse", "--path-format=absolute", "--git-common-dir")
    _, gitdir, _ = git(d, "rev-parse", "--path-format=absolute", "--git-dir")
    _, branch, _ = git(d, "branch", "--show-current")
    return True, (bool(common) and common == gitdir), branch


def git_segments(cmd, verb_pat):
    """逐个简单命令段(以 | & ; 换行 切分)找出同段含 `git` 且匹配 verb_pat 的段,
    避免 echo/heredoc 里仅"提到"命令造成误拦。返回匹配到的段列表。"""
    segs = []
    for seg in re.split(r"[|&;\n]", cmd):
        if re.search(r"\bgit\b", seg) and re.search(verb_pat, seg):
            segs.append(seg)
    return segs


def main():
    if os.environ.get("AUTOFORGE_GUARD_OFF") == "1":
        sys.exit(0)
    try:
        data = json.load(sys.stdin)
    except Exception:
        sys.exit(0)
    tool = data.get("tool_name", "")
    ti = data.get("tool_input", {}) or {}
    cwd = data.get("cwd") or os.getcwd()

    # ---- Write / Edit / NotebookEdit:保护主仓 <dev> 只读镜像 ----
    if tool in ("Write", "Edit", "NotebookEdit"):
        fp = ti.get("file_path") or ti.get("notebook_path") or ""
        target_dir = os.path.dirname(fp) if os.path.isabs(fp) else cwd
        inside, is_main, branch = repo_info(target_dir or cwd)
        if inside and is_main and branch == DEV:
            deny(f"主仓 {DEV} 是只读镜像,禁止在此编辑(BRANCHING.md §1)。请先 "
                 f"`bash scripts/branch/wt-new.sh <任务名>` 建 worktree,再 EnterWorktree 切入后编辑。"
                 f"(确需在主仓维护本机制:以 AUTOFORGE_GUARD_OFF=1 启动 Claude)")
        sys.exit(0)

    if tool != "Bash":
        sys.exit(0)

    cmd = ti.get("command", "") or ""
    # 放行我们自己的脚本(其内部已遵守规范)
    if re.search(r"scripts/branch/(land|wt-new|wt-clean)", cmd):
        sys.exit(0)

    _, is_main, branch = repo_info(cwd)
    dev_re = rf"\b(origin/)?{re.escape(DEV)}\b"

    # force / 破坏性 push
    for seg in git_segments(cmd, r"\bpush\b"):
        if re.search(r"--force(-with-lease)?\b|\s-f\b", seg) or re.search(r"\s\+[\w/]+:", seg):
            deny("禁止 force/破坏性 push(BRANCHING.md §6)。")
        if re.search(r"\b(main|master)\b", seg):
            deny("禁止 push 到 main/master(BRANCHING.md §6)。")
        if re.search(dev_re, seg):
            deny(f"不要直接 push 到 {DEV}。请用 `bash scripts/branch/land.sh`"
                 f"(rebase→push,被拒自动重试,保证无双头)。")
    # 裸 pull(既非 --ff-only 也非 --rebase)
    for seg in git_segments(cmd, r"\bpull\b"):
        if not re.search(r"--ff-only|--rebase", seg):
            deny(f"裸 git pull 会产生 merge 提交/双头。请用 `git pull --ff-only`(镜像追新)"
                 f"或 `git fetch && git rebase origin/{DEV}`(worktree 同步)。")
    # merge dev → 应 rebase
    for seg in git_segments(cmd, r"\bmerge\b"):
        if re.search(dev_re, seg) and not re.search(r"merge\s+--(continue|abort|quit)", seg):
            deny(f"不要 merge {DEV}(会污染线性历史)。请用 `git rebase origin/{DEV}`。")
    # 在 dev 分支上 commit
    if branch == DEV and git_segments(cmd, r"\bcommit\b"):
        deny(f"{DEV} 是集成主干,不在其上直接提交。请先建 feature worktree"
             f"(bash scripts/branch/wt-new.sh <任务名>)再提交。")
    # 在主仓把 dev 切走
    if is_main and branch == DEV:
        for seg in git_segments(cmd, r"\b(switch|checkout)\b"):
            names_dev = re.search(rf"(switch|checkout)\b[^|;&]*{re.escape(DEV)}\b", seg)
            looks_flagish = re.search(r"(switch|checkout)\b[^|;&]*(-{1,2}\w|\.)", seg)
            if not names_dev and not looks_flagish:
                deny(f"主仓需保持签出 {DEV}(维持 AutoForge 安全合并路径)。开发请用隔离 worktree,"
                     f"勿把主仓切到其它分支。")

    sys.exit(0)


if __name__ == "__main__":
    main()
