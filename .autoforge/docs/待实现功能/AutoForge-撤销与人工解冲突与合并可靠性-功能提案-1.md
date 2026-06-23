# 稳定撤销 issue 改动 + 人工解冲突器 + 合并可靠性加固

| 字段 | 值 |
|------|----|
| 状态 | 📝 待实现（2026-06-20 成文；基于合并/冲突状态机深度审计 + 撤销/解冲突两轮交互方案定稿） |
| 优先级 | P1（撤销是运维刚需；H1/H2/M1 为正确性缺陷，且是两特性共同前提） |
| 涉及层 | 后端（tasks·commands·core·models·state）+ DB（迁移 0058）+ 前端（Audit·services） |
| 工作量 | 中–大（P0 可靠性基座 1 天；P1 撤销 0.5 天；P2 解冲突器 1.5 天；P3 加固 0.5 天） |
| 相关 | `tasks/merge.rs`、`tasks/runner.rs`、`tasks/testing.rs`、`core/concurrency.rs`、`commands/change_requests.rs`、`state.rs`；配套任务清单 `撤销与人工解冲突与合并可靠性-tasks.json` |
| 长期对齐 | 撤销/解冲突逻辑全部下沉到不带 Tauri 类型的纯 async fn（CLAUDE.md 铁律 #1/#3），事件只走 `event::emit(AppEvent)`，为后端独立化铺路 |

---

## 1. 背景与问题

当前「出问题后撤销 issue 改动」与「人工解决合并冲突」两条运维动线都很薄弱，且审计发现合并状态机有若干正确性缺陷，三者相互咬合，合并为一份提案统一落地。

### 1.1 撤销能力缺口
- **从不记录合并产生的提交 SHA**：`merge.rs` 落地全程只产出提交，却不把生成的 commit SHA 落库（`worktree_sessions` 仅有 `base_commit` 分叉点与 `diff_content` 快照）。撤销的前提「知道撤哪个 commit」是空的。
- **无任何 revert/撤销命令**（后端 grep `revert|rollback|回滚` 零命中）。issue 合并进 dev、worktree 删除后，只能手动 git。

### 1.2 人工解冲突交互薄弱
- `merge_conflict` 态下（`Audit.tsx:2297-2330`）只提供：冲突文件名列表 + 一个 `<details>` **只读 dump** 带标记的 `conflict_diff` + 两个自动逃生按钮（AI 解 / 一键重试）。**应用内没有任何可操作的人工解决界面**，离 IDEA 三方编辑器差一整层。
- 后端从未用 `git show :1/:2/:3`（base/ours/theirs）三方 stage blob——三方视图的正确数据源缺位。
- 冲突检测后立即 `merge --abort`（`merge.rs:206/216`），冲突只以冻结文本快照存在，做交互式解决须按需重新物化。

### 1.3 审计发现的合并状态机缺陷（详见第 6 节）
- **H1**：spec 已更新为 `git merge --squash`（`testing.md:11` / `CLAUDE.md:205`），代码仍 `merge --no-ff`（`merge.rs:42-48` / `:91-98`）——规格↔实现分叉。
- **H2**：同一 CR worktree 上 `ai_resolve_conflict`（脱锁后台 spawn，`merge.rs:515` / `change_requests.rs:359`）与 `retry_merge` 触发的 `merge::run`（持项目锁）可**无锁并发**操作同一 worktree → git index 损坏。
- **M1**：`enqueue` 幂等键只防重复行、不防重复执行（`runner.rs:220-244` 无条件 `tx.send`）——撤销「靠幂等防双击」的假设不成立。

---

## 2. 目标 / 非目标

**目标**
- 每个 issue 在 dev 上对应**一个可撤销提交单元**；提供 `git revert` 前向撤销（不改写历史、不撞 GitProxy force-push 禁令、对已 push 的共享 dev 稳定）。
- 应用内**逐 hunk 决策式三方解决器**（IDEA-lite，决策优先非自由编辑）+ 结构性大冲突**外部 IDE 兜底**；AI/人工/外部三路共享同一条「提交→测试门→回代码审核」尾段。
- 修复 H1/H2/M1 等正确性缺陷，作为上述两特性的可靠性基座。

**非目标**
- 不引入完整三栏可编辑编辑器（Monaco/CodeMirror）——与手写 CSS/终端美学有张力，投入产出比低（方案 A，暂不做）。
- 撤销/解冲突撞冲突时**不自动改写**结果（MVP：abort + 如实上报，人工决断）。
- 不改 dev↔main 主流程、不动需求分析/执行链路。

---

## 3. 关键设计决策

1. **合并方式采纳 `--squash`（H1）**：`land_on_dev` 改 `merge --squash <branch>` + 显式 `commit -m <merge_msg>`，每个 CR 在 dev 上压成**一个普通提交**。撤销随之简化为 `git revert <sha>`（普通提交，**无需 `-m 1`**）。分支清理改 `branch -D`（squash 不标记已合并，`-d` 会拒绝）。
2. **撤销单元 = 该 CR 的 squash 提交 SHA**：合并成功后 `git rev-parse HEAD` 落 `worktree_sessions.merge_commit`（迁移 0058 加列；旧行 NULL → 撤销置灰降级）。
3. **撤销复用安全合并管道**：`revert_on_dev` 镜像 `land_on_dev` 双路径（in-place / 自管隔离-push），持 `merge_lock` 串行。
4. **撤销去重靠原子状态门、不靠幂等键（M1）**：触发前 `UPDATE … SET status='reverting' WHERE id=? AND status='merged'`，`rows_affected==0` 即拒绝（已撤/非法态）。
5. **冲突态按需物化**：解决器打开时若无 `MERGE_HEAD` 则在 CR worktree 重跑 merge 重建冲突（不 abort）；三方真源走 `git show :1/:2/:3`。
6. **三路共享尾段**：抽 `finalize_resolution()`（提交→`run_and_gate`→通过回 `pending_code_review`/失败 `merge_failed`），AI（现 `merge.rs:335-410`）/人工/外部统一调用。
7. **per-CR 互斥（H2）**：新增按 CR 的锁，覆盖所有写 worktree 的冲突操作 + `merge::run`，根治同 worktree 并发。
8. **撤销保留历史**：CR/issue 置 `reverted` 终态，保留 diff 快照与 CR/issue 行，留审计痕迹。

---

## 4. 实施方案（分阶段，详见 tasks.json）

### P0 合并可靠性基座（两特性共同前提）
- **H1** `land_on_dev` 改 squash + commit + `rev-parse HEAD` 记 `merge_commit`（迁移 0058 加列；`run()` 成功后回填）。
- **H2** `state.rs` 新增 `cr_lock(cr_id)`；`merge::run` 与所有 `ai_resolve_conflict` 入口持锁；命令入口加「仅 `merge_conflict` 可触发，触发即翻 `resolving_conflict`」防重入门。
- **M1** `enqueue` 对已存在且非 `pending/failed` 的 key 跳过 `send`；危险命令（merge/revert/resolve）一律加原子状态门。

### P1 稳定撤销 issue 改动
- `AppEvent::CrReverted{cr_id,project_id}`（+ `cr_id()` 匹配 + `notification.rs::from_event`）。
- `JobPayload::Revert{change_request_id}` + `runner.rs` 分发到 `tasks::revert::run`。
- `tasks/revert.rs`：校验 `merge_commit` 存在 + 状态门 `merged→reverting` → 持 `merge_lock`/`cr_lock` → 探测 `dev_is_live` → `revert_on_dev`（双路径 `git revert <sha>`）→ 成功置 `reverted`+emit；冲突 `revert --abort`+回 `merged`+如实上报。
- 命令 `revert_change_request(cr_id)`（薄包装：状态门 + 入队）；`lib.rs`+`capabilities` 注册；`services/index.ts` 封装。
- `Audit.tsx`：merged CR 详情底部「撤销此需求改动」`.btn-danger` + 二次确认弹窗（DESIGN：遮罩 `inset:var(--win-gutter)`、不点遮罩关闭、✕/Esc）；无 `merge_commit` 置灰降级；订阅 `CrReverted` 刷新。

### P2 人工解冲突器（B 逐 hunk 决策 + C 外部 IDE 兜底）
- `merge.rs`：抽共享 `finalize_resolution()`；新增 `materialize_conflict()`（MERGE_HEAD 幂等重建，不 abort）。
- `commands/conflicts.rs`（新模块，注册四步）：
  - `get_conflict_detail(cr_id)` → 物化 → `--diff-filter=U` 列文件 → 每文件切**有序段** `{type:context,lines}|{type:conflict,ours,theirs,base?}`，标记 `binary`/`delete_side`。
  - `resolve_conflict_manually(cr_id, files: Option<Map<path,content>>)` → 有值（应用内 B）先写盘、None（外部 C 已改盘）跳过 → `finalize_resolution`；长任务后台 spawn + 状态门。
  - `open_conflict_workspace(cr_id)` → 物化 + `core/platform` 打开 `worktree_path`。
- `Audit.tsx` `ConflictResolver` 面板替换只读 `<details>`：左冲突文件切换（每文件 已解/总）；右逐 hunk 卡片两列「dev（传入）/本分支（你的）」复用 `.diff-hunk`+code-bg（`Audit.tsx:114-137`），决策按钮 `采用本分支/dev/两者保留/手动编辑`；底 `确认解决并复审`（全决策后可点）+ `AI 解冲突`+`外部编辑器`+`我已在外部解决·复审`。前端按选择拼整文件回传。
- `services/index.ts` 三封装 + 类型。

### P3 审计余项加固
- **M2** land 阶段冲突归一到 `merge_conflict` 通道（捕获 `conflict_files/conflict_diff`），与 P2 解决器共用 UI。
- **M3** `sync_dev_into_worktree` 区分 `nothing-to-merge`(放行) 与其他非零(置错误态阻断)，不再一律当 clean（`merge.rs:205-207`）。
- **L1** `ai_resolve_conflict`/retry 回流 `pending_code_review` 补 `transition_to_pending_review()`（修内存计数漂移；准入用 DB COUNT 故仅显示层，`runner.rs:166-169`）。
- **L2** retry/merge 前校验 worktree 存在或重建，避免在 `repo_path` 错误路径上跑测试（`testing.rs:86-90`）。

---

## 5. 验收

- `cargo build`（src-tauri）+ `cargo test --lib` 全绿、新文件 clippy 零告警；`npm run build` 通过。
- `tasks/revert.rs` 单测：建仓 → squash 合并取 SHA → `git revert <sha>` → 断言改动被撤、文件回合并前；冲突用例断言 `revert --abort` 还原工作树。
- `conflicts.rs` 单测：造冲突 → `get_conflict_detail` 断言段结构 → `resolve_conflict_manually` 断言提交后无标记、回 `pending_code_review`。
- `tauri:dev` 手动走查：撤销动线（含旧 CR 置灰）、解冲突三路（应用内/AI/外部）、并发防重入。

---

## 6. 审计证据（合并/冲突状态机，2026-06-20）

| 级别 | 位置 | 问题 | 修复归属 |
|------|------|------|----------|
| 🟠H1 | `merge.rs:42/91` vs `testing.md:11` | spec=squash，代码=no-ff，分叉 | P0 |
| 🟠H2 | `merge.rs:515`/`change_requests.rs:359` | 同 worktree 无锁并发（ai_resolve × retry / 双击） | P0 |
| 🟡M1 | `runner.rs:220-244` | 幂等键不防重复执行（无条件 `tx.send`） | P0 |
| 🟡M2 | `merge.rs:657` | land 阶段冲突只置 merge_failed，无三方 UI | P3 |
| 🟡M3 | `merge.rs:205-207` | 非冲突 merge 失败被当 clean 放行 | P3 |
| 🔵L1 | `merge.rs:396`+`concurrency.rs:52` | 回流 pending_review 少计（仅显示漂移） | P3 |
| 🔵L2 | `testing.rs:86-90` | worktree 缺失时在 repo_path 错误路径跑测试 | P3 |
| 🔵L4 | `merge.rs` | merge commit SHA 从未记录（撤销地基缺口） | P0/P1 |

**缺失场景**（解冲突器须覆盖）：modify/delete 冲突（无标记）、二进制冲突、rerere 复用错误解法、空 diff CR 边界。

---

## 7. 风险与回退

- **squash 改造影响面**：分支不再被标记已合并，清理须 `-D`；若不愿改合并方式，回退 H1（保留 `--no-ff`，撤销改 `git revert -m 1 <sha>`），其余阶段不变。
- **撤不干净**：后续需求依赖被撤改动 → revert 冲突，MVP 只 abort+上报，交人工/外部解决。
- **物化冲突的副作用**：解决器打开会在 worktree 留下 MERGE 进行态；须保证幂等且与 per-CR 锁互斥，避免与 retry 撞车。
- **自管仓库自指**：撤销/解冲突在自管路径一律隔离 worktree + push origin/dev，绝不碰主工作树（沿用 `land_on_dev` 既有规避）。
