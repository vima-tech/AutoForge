#!/usr/bin/env bash
# wt-new <任务名> —— 从最新 origin/<dev> 建一棵隔离 worktree + feature 分支,打印其路径。
# 见 BRANCHING.md §3.1。一任务一棵,Claude session 切入后只碰这棵树。
set -euo pipefail
DEV="${AUTOFORGE_DEV_BRANCH:-dev}"

NAME="${1:-}"
[ -z "$NAME" ] && NAME="task-$(date +%m%d-%H%M%S)"
SLUG=$(printf '%s' "$NAME" | tr ' /' '--' | tr -cd 'A-Za-z0-9._-')
[ -z "$SLUG" ] && SLUG="task-$(date +%m%d-%H%M%S)"

# 始终基于主仓(从任意 worktree 调用都定位到同一个主 .git)
COMMON=$(git rev-parse --path-format=absolute --git-common-dir)
MAIN=$(dirname "$COMMON")
WTDIR="${AUTOFORGE_WT_DIR:-$(dirname "$MAIN")/$(basename "$MAIN")-wt}"
mkdir -p "$WTDIR"
DEST="$WTDIR/$SLUG"

if [ -e "$DEST" ]; then
  echo "✗ 目标已存在: $DEST(换个任务名,或先 wt-clean)" >&2
  exit 1
fi

git -C "$MAIN" fetch origin "$DEV" >&2
git -C "$MAIN" worktree add "$DEST" -b "feat/$SLUG" "origin/$DEV" >&2
echo "✓ 已创建 worktree: $DEST(分支 feat/$SLUG,基点 origin/$DEV)" >&2
echo "  接下来:EnterWorktree path=$DEST,之后所有编辑都在该树内。" >&2
# stdout 只输出纯路径,便于脚本/agent 捕获
printf '%s\n' "$DEST"
