# 并发合并冲突解决（串行合并 + 自动 rebase + AI 解冲突）

| 字段 | 值 |
|------|----|
| 状态 | 📝 待实施（方案已评审锁定，2026-06-19） |
| 优先级 | P0（L0 串行是竞态 bug，须先修）+ P1（L1 冲突处理） |
| 涉及层 | 后端（tasks/merge·state·core/gate·commands）+ DB（迁移 0049）+ 前端（Audit·Settings·services） |
| 工作量 | 中（Phase 0–1 约 1 天；Phase 2 UI 0.5 天；Phase 3 AI 解冲突 0.5–1 天） |
| 相关 | `src-tauri/src/tasks/merge.rs`、`src-tauri/src/tasks/runner.rs`、`src-tauri/src/core/gate.rs`、本目录 `并发合并冲突解决-tasks.json` |
| 配套修复 | CR diff 已改为对固定分叉点计算（base_commit + merge-base 回退，见 `change_requests.rs::compute_worktree_diff`），本方案与之同源问题 |

---

## 1. 背景与问题

AutoForge 是 Human-Lite-in-the-Loop 自主软件工厂，多个需求（CR）可并发执行（`concurrency.rs` 信号量默认 5）。
每个 CR 在独立 worktree 内从 `dev` 分叉作业，审核 2 通过（`change_requests.rs:376`）或门控自动放行（`execution.rs:565`）后入队 `JobPayload::Merge`，由 `tasks/merge.rs::run` 合并回 dev。当并发 CR 先后合并 dev 时，现实现有三个缺陷：

1. **合并不串行（竞态 bug）。** `runner.rs:28` 每个 job 各自 `tokio::spawn`，只有 **Execution** 走并发信号量，**Merge 不受限**。两个 CR 同时到 `pending_merge` 时，非自管项目走 `land_on_dev` 快路径（`merge.rs:34` `checkout dev && merge --no-ff` 直接操作主工作树），并发执行会互相踩同一个 dev 工作树 → 工作树损坏 / 合并交错。

2. **合并前不刷新 base。** CR 分支始终停在分叉时的旧 dev。先合的成功，后合的拿旧 base 撞新 dev，重叠文件即冲突。

3. **冲突即放弃。** `merge.rs:51/101` 一旦冲突就 `git merge --abort`，CR 置 `merge_failed`，报告写"常见原因为代码冲突，可修复后重新执行"——无自动解决、无三方对比 UI、无一键 rebase，全靠人重跑。

> 典型重现：ASR 需求分支 `a6c6f28` 与手动合并 `f21c245` 都改了 `Conversations.tsx`，先后落 dev 必冲突。

## 2. 目标 / 非目标

**目标**
- 同项目合并严格串行，消除并发踩工作树的竞态。
- 落地前自动把 dev 并入 CR 分支并重测，自动消化"纯文本冲突但语义无碍"的多数情况。
- 真冲突不再静默丢弃：保留冲突现场，提供人工三方视图 + 一键重试，以及可选的 AI 自动解冲突。
- AI 解冲突路径必经审核 2 复审兜底，不新开合并旁路。

**非目标**
- 不做跨项目全局串行（跨项目仍并行）。
- 不做文件作用域调度 / merge-queue（方案 C，留作并发量增大后的演进）。
- 不做 rebase 改写历史（默认 merge dev 入分支，保 rerere 友好、不打乱 worktree 提交）。

## 3. 决策（已评审锁定）

| 项 | 决策 |
|----|------|
| L0 串行 | 无条件先修；**按 project_id** 串行，跨项目并行 |
| L1 默认 | **方案 A**：合并前自动 `git merge <dev>` 入 CR 分支（**非 rebase**）+ `git rerere` + 重测 |
| B 保留 | 冲突态的**手动「AI 解冲突并合并」按钮**（"快速合并"） |
| 自动 B 开关 | 与"跳过人工审核"（门控降级/自动放行）开关**并排**放 `Settings.tsx` 门控降级面板 |
| 新状态 | `merge_conflict`（区别于 `merge_failed`；status 字段无 CHECK 约束，可自由加） |

## 4. 设计

### 4.1 数据模型（迁移 `0049_merge_conflict.sql`）
- `app_settings` 新键 `auto_conflict_resolve_enabled`，默认 `'false'`（沿用 `auto_pass_enabled` 同表同模式，见 `gate.rs:16/26`）。
- `worktree_sessions` 加列：
  - `conflict_files TEXT` — 冲突文件路径 JSON 数组。
  - `conflict_diff TEXT` — 带冲突标记的快照（供 UI 三方视图与 B 的 agent 输入）。
- CR/issue `status` 新增取值 `merge_conflict`（无需建表）。

### 4.2 Phase 0 — 串行合并
- `state.rs`：`AppState` 加纯字段 `merge_locks: Arc<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>`（无 Tauri 类型，符合解耦铁律）。
- `merge.rs::run`：加载 cr/project 后，按 `project_id` get-or-insert 取该项目的 `tokio::Mutex`，acquire 后再进入 rebase+测试+land 全流程，函数结束自动释放。

### 4.3 Phase 1 — 自动 merge-dev + 重测（持锁中，`land_on_dev` 之前）
1. `git fetch origin <dev>`（best-effort），确定 dev ref（本地或 `origin/<dev>`）。
2. CR worktree 内 `git config rerere.enabled true`。
3. CR worktree（`session.worktree_path`，agent 改动已由 `execution.rs` 提交、工作树干净）执行 `git merge <devref>`：
   - **干净** → 重测（4.3.4）→ 进入 `land_on_dev`（分支已含 dev，落地为快进式干净合并）。
   - **冲突** → `ls-files -u`/`diff` 取冲突文件+带标记快照写入 `conflict_files/conflict_diff` → `git merge --abort` 还原 → 置 CR/issue `merge_conflict` → **自动 B 开关 ON 则转 Phase 3，否则停在该态等人**。
4. **重测（新增轻量集成校验）**：用 `run_config::effective_config` / `stack` 画像里的测试命令在 worktree 跑；无配置则跳过（以 dev-merge 干净为主门禁）。失败 → 置 `merge_conflict`（原因"集成后测试失败"），不落地。

> 现状：仓库无独立 pre-merge 测试任务，测试是执行期 code agent 自跑（`merge.rs:376` 注释）。本重测为 best-effort 新增，不阻断无测试命令项目。

### 4.4 Phase 2 — 人工兜底 UI（审核页）
- 后端命令（`change_requests.rs` 薄包装 + 下层 async fn）：
  - `get_merge_conflict(cr_id) -> {files, diff}`：读 `conflict_files/conflict_diff`。
  - `retry_merge(cr_id)`：重新入队 `JobPayload::Merge`（走 Phase 1，dev 已前进时可能自动消解）。
- `lib.rs` 注册 + `services/index.ts` 封装。
- 前端 `Audit.tsx`：
  - 状态映射加 `merge_conflict`（label「合并冲突」、color `amber`、`STATUS_ORDER`/`FAILED_STATUSES`，行 52/65/71/74/483/487）；`Dashboard.tsx:24/39` 同步。
  - `merge_conflict` 态详情：三方冲突视图（复用既有 diff 渲染 + `.panel`，只用 CSS 变量）+「重试合并」按钮。

### 4.5 Phase 3 — B：手动 AI 解冲突 + 自动开关
- `core/gate.rs`：加 `auto_conflict_resolve_enabled/set_...`（镜像 `auto_pass_enabled`）。
- `commands/grading.rs`（或 settings.rs）：加 `get/set_auto_conflict_resolve_enabled` 命令；`lib.rs` 注册；services 封装。
- `ai_resolve_conflict(cr_id)`（手动按钮 / Phase 1 自动触发）：worktree 重做 dev-merge → `conflict_diff`+两边意图喂 `code_agent` 消解 → `git add`+commit → 复跑测试 → **回 `review_2`（pending_review_2）复审**，不直接落 dev。
- 前端：
  - `Settings.tsx` 门控降级面板（`:1906` panel 内）新增并排 toggle「冲突时自动 AI 解冲突合并」，绑 `get/set_auto_conflict_resolve_enabled`，沿用 `autoPassOn` toggle 写法与 `.panel-head`/`chip`/`btn-sm` 样式。
  - `Audit.tsx` `merge_conflict` 态加「AI 解冲突并合并」按钮（调 `ai_resolve_conflict`）。

## 5. 约束遵从（CLAUDE.md / DESIGN.md）
- AI 解冲突结果回灌前过 `core/security::has_obvious_injection()`（agent 输出视为外部输入）。
- 合并唯一入口仍是 `review_2 approved` / 门控放行；B 路径**必经 review_2 复审**，不新开旁路。
- 所有 git 操作走 `GitProxy`；事件只走 `event::emit(AppEvent)`，新增 `MergeConflict` 变体。
- 业务逻辑零 Tauri 类型（`state.rs` 新字段为纯类型）；command 薄包装；迁移仅追加（0049）。
- 前端只用 CSS 变量、`<Icon>`、自定义下拉；每屏 ≤1 主按钮；IPC 只走 services 层。

## 6. 测试与验收
- 串行：两个同项目 `pending_merge` CR 不并发 checkout dev。
- A：改同一文件的两 CR，先合成功；后合自动 merge-dev，干净则合、冲突则 `merge_conflict` 且现场已记录。
- 兜底：`retry_merge` 在 dev 含解后能消解；三方视图正确展示冲突。
- B：自动开关 ON 时冲突走 agent→复跑测试→回 review_2；手动按钮同路径。
- 回归：非冲突类 `merge_failed`（如 push 失败）行为不变。
- IPC/窗口相关走 `npm run tauri:dev` 实测。

## 7. 交付顺序
Phase 0（串行，纯 bug 修，可独立验证）→ Phase 1（自动 merge-dev+重测）+ 迁移 0049 + 状态机 → Phase 2（兜底 UI）→ Phase 3（B 手动按钮 + 自动开关）。
