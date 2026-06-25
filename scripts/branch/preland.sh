#!/usr/bin/env bash
# preland —— 落地前强校验。全部通过才允许 land(land 会在 rebase 后自动调用本脚本)。
# 也可独立随时自检。只 fetch、只读:不 rebase、不 push、不改远端。见 BRANCHING.md §5。
set -uo pipefail
DEV="${AUTOFORGE_DEV_BRANCH:-dev}"
fail=0
note(){ printf '%s\n' "$*"; }
bad(){ printf '  ✗ %s\n' "$*"; fail=1; }
ok(){ printf '  ✓ %s\n' "$*"; }

note "── preland 落地前强校验 ──"
git rev-parse --is-inside-work-tree >/dev/null 2>&1 || { bad "不在 git 仓库内"; exit 1; }

# 1) 当前分支不是 dev、非 detached
CUR=$(git branch --show-current)
if [ -z "$CUR" ]; then bad "处于 detached HEAD"
elif [ "$CUR" = "$DEV" ]; then bad "当前在 $DEV,应在 feature 分支落地"
else ok "分支 $CUR(非 $DEV)"; fi

# 2) 工作树干净、改动全部已提交
if [ -n "$(git status --porcelain)" ]; then bad "工作树有未提交改动,请先 commit"; else ok "工作树干净、已全部提交"; fi

# 3) 无冲突标记
if git grep -nI -e '^<<<<<<<' -e '^>>>>>>>' -- . >/dev/null 2>&1; then
  bad "检测到冲突标记 <<<<<<< / >>>>>>>:"; git grep -nI -e '^<<<<<<<' -e '^>>>>>>>' -- . | sed 's/^/    /'
else ok "无冲突标记"; fi

# 4) 无空白/标记错误
if git diff --check HEAD >/dev/null 2>&1; then ok "git diff --check 通过"
else note "  ⚠️ git diff --check 报告问题(空白/标记):"; git diff --check HEAD | sed 's/^/    /' || true; fi

# 5) 已基于最新 origin/dev
git fetch origin "$DEV" >/dev/null 2>&1 || true
if git rev-parse --verify -q "origin/$DEV" >/dev/null; then
  if git merge-base --is-ancestor "origin/$DEV" HEAD; then ok "已基于最新 origin/$DEV"
  else bad "落后 origin/$DEV,请先 git rebase origin/$DEV(land 会自动 rebase)"; fi
fi

# 6) 高冲突文件预警(相对 origin/dev 的改动范围)——见 BRANCHING.md「冲突域控制规则」
BASE=$(git merge-base "origin/$DEV" HEAD 2>/dev/null || echo "")
if [ -n "$BASE" ]; then
  HOT=$(git diff --name-only "$BASE"..HEAD 2>/dev/null | grep -E \
    'migrations/|(^|/)(package-lock\.json|pnpm-lock\.yaml|yarn\.lock|Cargo\.lock)$|src/services/index\.ts|src/App\.tsx|src-tauri/src/lib\.rs|src-tauri/src/state\.rs|(^|/)mod\.rs$|src/index\.css' || true)
  if [ -n "$HOT" ]; then
    note "  ⚠️ 触及【高冲突文件】,确认无其它 session 并行修改同一文件(否则串行/先协调):"
    printf '%s\n' "$HOT" | sed 's/^/      - /'
  fi
fi

# 7) 可选测试/lint(设 AUTOFORGE_PRELAND_CMD 启用,如 "npm run lint && cargo check")
if [ -n "${AUTOFORGE_PRELAND_CMD:-}" ]; then
  note "  → 运行校验命令: $AUTOFORGE_PRELAND_CMD"
  if bash -c "$AUTOFORGE_PRELAND_CMD"; then ok "测试/lint 通过"; else bad "测试/lint 失败"; fi
else
  note "  ℹ️ 未设 AUTOFORGE_PRELAND_CMD,跳过测试/lint(强烈建议设置)"
fi

if [ "$fail" = 0 ]; then note "✅ preland 全部通过"; exit 0
else note "❌ preland 未通过,请修复后再 land"; exit 1; fi
