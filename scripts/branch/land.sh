#!/usr/bin/env bash
# land —— 把当前 feature 分支落到 origin/<dev>:preland 强校验 + rebase-then-push + 拒绝即重试。
# 见 BRANCHING.md §4/§5。无锁,靠 git 原子 ref 更新天然串行化,永不双头。
set -euo pipefail
DEV="${AUTOFORGE_DEV_BRANCH:-dev}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

git rev-parse --is-inside-work-tree >/dev/null 2>&1 || { echo "✗ 不在 git 仓库内"; exit 1; }
CUR=$(git branch --show-current)
[ -z "$CUR" ] && { echo "✗ 处于 detached HEAD,无法 land"; exit 1; }
[ "$CUR" = "$DEV" ] && { echo "✗ 你在 $DEV 上,land 必须从 feature 分支运行"; exit 1; }
# 快速结构检查:工作树必须干净,否则 rebase 会失败/丢改动
[ -n "$(git status --porcelain)" ] && { echo "✗ 工作树有未提交改动,请先 commit 再 land"; exit 1; }

echo "→ land 分支 '$CUR' → origin/$DEV"
git fetch origin "$DEV"
if ! git rebase "origin/$DEV"; then
  echo "⚠️ 与 origin/$DEV 冲突。解决后执行: git rebase --continue,再重跑 land。"
  exit 1
fi

# 落地前强校验(在已 rebase 的结果 = 集成后状态上跑)
if ! bash "$HERE/preland.sh"; then
  echo "✗ preland 未通过,已中止 land(未 push)。修复后重跑 land。"
  exit 1
fi

for i in 1 2 3 4 5; do
  if git push origin "HEAD:$DEV"; then
    echo "✅ 已落地 origin/$DEV(分支 $CUR)。确认远端无误后再 'bash scripts/branch/wt-clean.sh' 清理本 worktree。"
    exit 0
  fi
  echo "↻ origin/$DEV 又被推进,自动 fetch+rebase 重试 ($i/5)…"
  git fetch origin "$DEV"
  if ! git rebase "origin/$DEV"; then
    echo "⚠️ 重试 rebase 时冲突。解决后执行: git rebase --continue,再重跑 land。"
    exit 1
  fi
  # dev 已变,重跑强校验(尤其测试)再推
  if ! bash "$HERE/preland.sh"; then
    echo "✗ rebase 后 preland 未通过,已中止 land(未 push)。修复后重跑。"
    exit 1
  fi
done
echo "✗ 连续 5 次仍被抢先,origin/$DEV 推进过于频繁,请稍后重跑 land。"
exit 1
