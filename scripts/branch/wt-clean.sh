#!/usr/bin/env bash
# wt-clean —— 清理已落地/未使用的个人 worktree:工作树干净且相对 origin/<dev> 零提交才删,
# 绝不删主仓、绝不删有未提交改动或有未落地提交的树。见 BRANCHING.md §3.4。
set -euo pipefail
DEV="${AUTOFORGE_DEV_BRANCH:-dev}"

COMMON=$(git rev-parse --path-format=absolute --git-common-dir)
MAIN=$(dirname "$COMMON")
git -C "$MAIN" fetch origin "$DEV" >/dev/null 2>&1 || true

git -C "$MAIN" worktree list --porcelain | awk '/^worktree /{print $2}' | while read -r WT; do
  [ "$WT" = "$MAIN" ] && continue
  BR=$(git -C "$WT" branch --show-current 2>/dev/null || echo "")
  DIRTY=$(git -C "$WT" status --porcelain 2>/dev/null || echo "x")
  AHEAD=$(git -C "$WT" rev-list --count "origin/$DEV..HEAD" 2>/dev/null || echo "?")
  if [ -z "$DIRTY" ] && [ "$AHEAD" = "0" ]; then
    echo "🧹 prune $WT ($BR)"
    git -C "$MAIN" worktree remove "$WT"
    [ -n "$BR" ] && git -C "$MAIN" branch -D "$BR" 2>/dev/null || true
  else
    echo "✋ keep  $WT (分支=$BR, 未落地提交=$AHEAD, 有改动=$([ -n "$DIRTY" ] && echo 是 || echo 否))"
  fi
done
