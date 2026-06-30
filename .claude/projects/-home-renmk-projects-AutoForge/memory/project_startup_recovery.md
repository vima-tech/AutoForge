---
name: project_startup_recovery
description: 崩溃/重启的断点续传全覆盖——各 job 在途态恢复策略 + 幂等性推理（哪些可自动重跑、哪些只能回滚）
metadata:
  type: project
---

进程内 Tokio 任务队列重启即丢，所有「在途但无活任务收尾」的状态都靠 `lib.rs:108` 启动恢复（任何 driver 任务产生前异步跑一次）自愈。

恢复函数与策略（关键是**幂等性**决定能否自动重跑）：
- **execution**：`runner::requeue_orphaned_executions` — `executing`→`pending_execution`+清旧 worktree，重新 fork 入队（任务级整体重试，非进程内断点续跑）。
- **analysis**：`requeue_orphaned_analyses` — 恢复 `pending_analysis`（analysis.rs 无 `analyzing` 中间态，全程停 pending_analysis 直到终态，故此一态全覆盖）。
- **merge**：`requeue_orphaned_merges` — 恢复 `pending_merge`，**可安全自动重排**：Merge 幂等，`git merge --squash` 已落地分支再跑是空操作；最坏丢 merge_commit SHA（merge 路径已容忍存 NULL）。三个置 pending_merge 的源：review_2 approved / 自动合并门 / 解冲突回落。
  - 重入细节：`land_on_dev` fast-path（普通项目 checkout dev+squash+commit）若崩在「squash 已暂存未 commit」之间，dev 索引残留半 squash 且**无 MERGE_HEAD（`merge --abort` 救不了）**，重跑 `merge --squash` 会因脏索引失败。已在 checkout dev 后加 `reset --hard HEAD` 清残留（只丢未提交残留、不动历史）使其可重入。隔离 path（self-managed）每次用全新 throwaway worktree，本就可重入。
  - 解冲突（AI 自动/手动）期间 CR 在 DB 仍是 `merge_conflict`（"resolving_conflict" 只是事件 phase 非状态），持锁重检去重，崩溃停 parked 态等人重试——刻意不自动重跑（code agent 有副作用/费用）。
- **revert**：`recover_orphaned_reverts` — `reverting`**回滚到 `merged`**，**绝不自动重跑**：`git revert` 不幂等，崩在「已 revert 未标 reverted」之间重跑会 revert 掉 revert（重新应用改动）。撤销本是人工动作，让用户确认 dev 后手动重试。
- **会议室任务**：`orchestration::fail_orphaned_conversation_tasks` — `running`→`failed`（连 steps/runs），**不自动重跑**：交互式、有副作用（发 AI 消息/扣 token/写工作区文件），重跑会重复。让用户重发指令。

刻意不恢复：`merge_conflict`/`merge_failed`/`*_failed` 是等人/AI 的 parked 终态；SecurityAudit 合并后非阻塞（CR 已 merged）；Testing job 已无人 enqueue（测试逻辑内联进 merge）。

判据：新增任何会让 issue/CR/task 停在「只能由内存任务推进」的中间态时，必须在启动恢复里加一条；幂等的自动重排，不幂等的回滚到上一稳定态让人重试。相关 [[project_self_managed_merge]] [[project_merge_conflict]]。
