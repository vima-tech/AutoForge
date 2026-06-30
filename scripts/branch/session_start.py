#!/usr/bin/env python3
"""SessionStart 钩子:刷新 origin/<dev>,并按当前位置向 agent 注入分支工作流指引。
见 BRANCHING.md。输出 hookSpecificOutput.additionalContext。"""
import json, os, subprocess, sys

DEV = os.environ.get("AUTOFORGE_DEV_BRANCH", "dev")


def git(cwd, *args, timeout=15):
    try:
        r = subprocess.run(["git", "-C", cwd, *args], capture_output=True,
                           text=True, timeout=timeout)
        return r.returncode, r.stdout.strip(), r.stderr.strip()
    except Exception:
        return 1, "", ""


def main():
    try:
        data = json.load(sys.stdin)
    except Exception:
        data = {}
    cwd = data.get("cwd") or os.getcwd()

    code, _, _ = git(cwd, "rev-parse", "--is-inside-work-tree")
    if code != 0:
        return  # 非 git 仓库,静默

    git(cwd, "fetch", "origin", DEV)  # best-effort 刷新远端

    _, common, _ = git(cwd, "rev-parse", "--path-format=absolute", "--git-common-dir")
    _, gitdir, _ = git(cwd, "rev-parse", "--path-format=absolute", "--git-dir")
    is_main = bool(common) and common == gitdir
    _, branch, _ = git(cwd, "branch", "--show-current")
    _, behind, _ = git(cwd, "rev-list", "--count", f"HEAD..origin/{DEV}")
    behind = behind or "?"

    lines = ["【分支工作流 · 自动托管】完整规范见仓库 BRANCHING.md。"]
    if is_main and branch == DEV:
        lines += [
            f"⚠️ 当前在【主仓 {DEV} 只读镜像】(落后 origin/{DEV} {behind} 个提交)。",
            f"主仓 {DEV} 是只读镜像,禁止在此编辑/提交(harness 会硬拦截)。",
            "▶ 若本次会话要改代码:第一步执行",
            "    bash scripts/branch/wt-new.sh <简短任务名>",
            "  它会基于 origin/" + DEV + " 建 feature 分支并打印 worktree 路径;",
            "  随后立即用 EnterWorktree(path=<该路径>) 切入,之后所有编辑都在该 worktree。",
            "▶ 纯问答/只读会话可留在主仓,但不要编辑或提交。",
            f"▶ 想看最新成果:git pull --ff-only origin {DEV}(永远 ff-only)。",
        ]
    elif not is_main:
        lines += [
            f"当前在隔离 worktree(分支 {branch or '?'},落后 origin/{DEV} {behind} 个提交)。",
            f"▶ 勤 rebase:提交前 / 落地前 / 发现 origin/{DEV} 更新即 "
            f"`git fetch origin && git rebase origin/{DEV}`(不要每天才一次)。",
            "▶ 完工落地:bash scripts/branch/land.sh"
            "(内置 preland 强校验 → rebase → push,被拒自动重试)。",
            "▶ 想先自检:bash scripts/branch/preland.sh。",
            f"▶ 切勿 git merge {DEV} / 裸 git pull / 直接 git push 到 {DEV}(用 land)。",
            "▶ 改到 migration/lockfile/lib.rs/state.rs/services/index.ts 等高冲突文件前,"
            "先确认无其它 session 并行修改(见 BRANCHING.md 冲突域规则)。",
        ]
    else:
        lines += [
            f"当前签出 {branch or '?'}(非 {DEV})。按 BRANCHING.md:开发请走隔离 worktree + feature 分支。",
        ]

    out = {"hookSpecificOutput": {"hookEventName": "SessionStart",
                                  "additionalContext": "\n".join(lines)}}
    print(json.dumps(out, ensure_ascii=False))


if __name__ == "__main__":
    main()
