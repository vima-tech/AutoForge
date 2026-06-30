# 代码 Agent 可插拔（claude / codex / opencode 互换）

| 字段 | 值 |
|------|----|
| 状态 | ✅ 已实现（2026-06-20；cargo test 107 passed、clippy 新文件零告警、npm run build 通过；P0–P3 全落地）。实测结论：① `GIT_ALLOW_PROTOCOL=""` 把 `git push` 拦在网络之前（`传输 'https' 不允许`），本地 commit 不受影响——对仅靠该护栏的 opencode 关键；② codex `-s workspace-write`/`-C`/`--skip-git-repo-check` 与 opencode `run --dir` 均经各自 CLI 校验接受。实际迁移序号 **0057**（0053–0056 已占用）。优化：Settings 编辑表单加 kind 选择器（可建 codex/opencode 自定义变体，改 kind 自动同步 program）+ 后端禁止禁用当前默认 agent。待运行期微调：codex/opencode 真实 CR 跑通与 auth 探测精度（需 tauri:dev + 登录态）。 |
| 优先级 | P2（架构解耦，非阻塞 bug；为后端独立化愿景铺路） |
| 涉及层 | 后端（agents·tasks·commands·models）+ DB（迁移 0057）+ 前端（Settings·services·Projects） |
| 工作量 | 中（P0 重构 0.5 天；P1 配置链路 0.5 天；P2 接两个 agent 1 天；P3 per-project 0.5 天） |
| 相关 | `src-tauri/src/agents/code_agent.rs`、`src-tauri/src/agents/local_claude.rs`、`tasks/execution.rs:341`、`tasks/merge.rs:322`、本目录 `代码Agent可插拔-tasks.json` |
| 长期对齐 | 呼应 CLAUDE.md「后端独立化 + MCP」愿景——code agent 抽象层零 Tauri 类型，未来可在非 Tauri 入口复用 |

---

## 1. 背景与问题

AutoForge 把代码实现交给本地 `claude` CLI 在隔离 worktree 内执行。当前对 claude CLI 的耦合分两层，**性质不同**：

- **层 A — 重型「代码实现 agent」（本提案目标）**：`agents/code_agent.rs::run()` spawn
  `claude --print --permission-mode acceptEdits --disallowedTools "Bash(git *)"`（prompt 走 stdin，cwd=worktree）。
  仅 **2 个调用点**：`tasks/execution.rs:341`（实现需求）、`tasks/merge.rs:322`（AI 解合并冲突）。
  配套 `code_agent::build_prompt()`（组 prompt）、`extract_report()`（抠 `## 改动摘要`）。

- **层 B — 轻型「文本 LLM」回退**：`agents/local_claude.rs`，跑纯文本 `claude --print`，用于
  `safety_check`、`analysis.rs` 分析回退、`llm.rs::run_agent_text` 中 **agent 无 `llm_id` 时的兜底**。

层 B **已自带解耦接缝**（有 `llm_id` 走自定义 LLM，否则才回退 claude），给相关角色绑 LLM 配置即可旁路，
**本提案不动层 B**。痛点集中在层 A：代码实现 agent 被写死为 claude，无法换用其它编码 CLI（codex / opencode），
也无法按项目选择。

> 长期价值：把 code agent 做成「可替换的执行适配层」，与 CLAUDE.md「Tauri 是薄壳不是地基」「后端独立化」一脉相承。

## 2. 目标 / 非目标

**目标**
- 抽出纯 Rust 的 `CodeAgent` 抽象（零 Tauri 类型），把现有 claude 包成首个实现，行为零变化。
- 用**单一配置驱动的 `CliCodeAgent`** 支持三种 kind：`claude` / `codex` / `opencode`，新增 agent 优先填配置而非写新类。
- 设置页可管理 code agent（启用/默认/program 路径/model/额外 flag）并查看健康（auth）状态。
- 支持 per-project 选择（不同项目可用不同 agent），缺省回落全局默认。
- **所有 agent 共享同一组安全不变量**（传输层禁 remote git、worktree 隔离、输出过注入检测），由调用层统一施加。

**非目标**
- 不动层 B 文本 LLM 回退（已有 `llm_id` 接缝）。
- 不支持「非 CLI / 纯 API 式」code agent（抽象暂以「进程」为粒度；如需，后续再抽一层，见 §8）。
- 不做 agent 自动择优 / 多 agent 竞速（留作演进）。

## 3. 三种 agent 调用矩阵（本地实测）

实测版本：claude 2.1.183、codex-cli 0.139.0、opencode 1.16.0。

| 维度 | **claude** | **codex** | **opencode** |
|---|---|---|---|
| 非交互入口 | `claude --print` | `codex exec` | `opencode run` |
| prompt 喂法 | **stdin** | arg 或 `-`（stdin） | arg（positional message） |
| 工作目录 | `current_dir()` | `-C/--cd <wt>` | `--dir <wt>` |
| 模型 | `--model <m>` | `-m <m>` | `-m <provider/model>` |
| 图片 | `--image <p>` | `-i <p>` | `-f <p>` |
| 自动改文件 | `--permission-mode acceptEdits` | `-s workspace-write` | 默认（无审批门） |
| 禁 remote git | `--disallowedTools "Bash(git *)"` | sandbox 默认断网 ✅ | **无 flag** → 靠 env |
| 报告约定 | prompt 要求 `## 改动摘要` | 共用同一 prompt | 共用同一 prompt |
| auth 探测 | `claude auth status` | `codex login` / `doctor`（无干净 status） | `opencode auth list` |
| 退出码 | 0=成功 | 0=成功 | 0=成功 |

**两个关键结论**
1. codex 的 `workspace-write` sandbox 默认断网，天然禁掉 remote git（安全加分）；opencode **无工具级禁用**，
   必须靠**传输层 env**（`GIT_ALLOW_PROTOCOL=""`）兜底。→ 把 git 禁用做成**所有 adapter 强制套的通用护栏**，
   而非依赖每家 flag（更可靠、agent 无关）。
2. 三者形状高度相似（prompt±cwd±model±image±退出码），可用**一个配置驱动的 `CliCodeAgent`** 覆盖，
   `kind` 只切「prompt 喂法 / 图片 flag / 权限 flag / git 禁用方式」少数分支。

## 4. 设计

### 4.1 数据模型（迁移 `0057_code_agents.sql`）

```sql
CREATE TABLE code_agents (
  id              TEXT PRIMARY KEY,
  kind            TEXT NOT NULL,          -- 'claude' | 'codex' | 'opencode'（决定内置 adapter 分支）
  label           TEXT NOT NULL,
  program         TEXT NOT NULL,          -- 可执行名/绝对路径（默认同 kind）
  model           TEXT,                   -- 可空，映射各家 --model/-m
  extra_args_json TEXT NOT NULL DEFAULT '[]',  -- 用户追加 flag（JSON 字符串数组）
  enabled         INTEGER NOT NULL DEFAULT 1,
  is_default      INTEGER NOT NULL DEFAULT 0,
  created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
-- 种子三条：claude(is_default=1) / codex / opencode，program 同 kind，model 留空
ALTER TABLE projects ADD COLUMN code_agent_id TEXT;  -- NULL = 用全局默认
```

`extra_args_json` 不含密钥，**不需 secrets 加密**（与 LLM api_key 不同）。

**选择优先级**：`projects.code_agent_id`（且 enabled）→ 全局 `is_default`（且 enabled）→ 硬兜底 `claude`。

### 4.2 Rust 架构（adapter 层零 Tauri 类型，遵守 CLAUDE.md 铁律 #1）

```
agents/code_agent/
  mod.rs    # trait CodeAgent + 共享 build_prompt/extract_report（迁自旧 code_agent.rs）+ resolve()
  cli.rs    # CliCodeAgent（配置驱动，一个 struct 覆盖三种 kind）
```

```rust
#[async_trait]
pub trait CodeAgent: Send + Sync {
    /// 在 worktree 内执行，返回 (exit_code, stdout, stderr)。签名与现有调用点一致。
    async fn run(&self, worktree: &str, prompt: &str, timeout: u64) -> Result<(i32, String, String)>;
    /// 该 agent 是否已安装并登录。
    async fn check_auth(&self) -> bool;
    fn kind(&self) -> &str;
}

pub struct CliProfile {
    pub kind: String,            // claude|codex|opencode
    pub program: String,
    pub model: Option<String>,
    pub extra_args: Vec<String>,
}
pub struct CliCodeAgent { profile: CliProfile }
```

- **一个 `CliCodeAgent` 吃下三种**；`kind` 决定 prompt 喂法（stdin vs arg）、cwd flag、image flag、权限 flag、git 禁用方式。
- `build_prompt` / `extract_report` **保持共享自由函数**：统一要求所有 agent 输出 `## 改动摘要`；
  `extract_report` 在 marker 缺失时**退化为返回全文**（codex/opencode 不保证乖乖输出标题）。
- `resolve(db, project) -> Box<dyn CodeAgent>` 是**唯一解析入口**；`execution.rs` / `merge.rs` 改调它。
- 旧 `agents/code_agent.rs` 内容迁入 `agents/code_agent/`，对外 API（`build_prompt`/`extract_report`/`run`）
  保持可用以减少改动面（P0 阶段 `run` 转调 `resolve(...).run()`）。

### 4.3 安全不变量（由 `CliCodeAgent::run` 对所有 kind 统一施加）

| 不变量 | 实现 |
|---|---|
| 禁 remote git（传输层，agent 无关） | `GIT_ALLOW_PROTOCOL=""` + `GIT_TERMINAL_PROMPT=0` env，**所有 kind 强制** |
| worktree 隔离 | `current_dir = worktree`，绝不在主工作树跑 |
| 进程组隔离 | 沿用 `core::platform::detach_process_group`（防 SIGTRAP 串扰） |
| 输出回灌前过注入检测 | `core::security::has_obvious_injection`（merge.rs 已有，统一到调用层） |
| 各家额外护栏 | claude: `--disallowedTools "Bash(git *)"`；codex: `-s workspace-write --skip-git-repo-check` |

这五条写在调用/适配层，**新增第四种 agent 自动继承**，不依赖 adapter 自觉。

### 4.4 命令与前端

后端新增 `commands/code_agents.rs`（薄包装，调纯 async fn）：
- `list_code_agents()` / `upsert_code_agent(payload)` / `delete_code_agent(id)`
- `set_default_code_agent(id)` / `set_project_code_agent(project_id, code_agent_id?)`
- `check_code_agent_auth(id)`（泛化现有 `check_claude_auth`；后者保留为兼容别名，内部转 `kind=claude`）

前端：
- `services/index.ts` 加对应 `ipc<T>()` 封装（**禁页面直接 invoke**）。
- `Settings.tsx` 新增「代码 Agent」区块：`proj-select` 选全局默认；每条 agent 的 program/model/启用开关 + 健康灯
  （复用现有 auth 检查 UI 样式）。**禁原生 select，用 `proj-select + mention-pop + mention-row`**；每屏 ≤1 主按钮。
- `Projects.tsx`（P3）：per-project 覆盖下拉，缺省「跟随全局默认」。
- 全部样式只用 `src/index.css` CSS 变量，图标走 `<Icon/>`。

## 5. 验收

- P0：`cargo build` 通过，默认仍走 claude，现有需求实现 + AI 解冲突流程零回归。
- P1：设置页可见三条 agent、可切默认；选 claude 仍正常跑通一个真实 CR。
- P2：分别切到 codex / opencode 各跑通一个真实 CR，验证 ① 能改 worktree 文件 ② **推不了 remote**（构造一次 `git push` 应失败）③ 报告可解析（有摘要走摘要，无则全文）④ 健康灯正确反映 auth。
- P3：两个项目分别绑不同 agent，并行各跑通一个 CR。
- 验证环境：涉 IPC/进程/文件，须在 `npm run tauri:dev` 完整环境走查。

## 6. 风险 / 未决

| 风险 | 缓解 |
|---|---|
| codex/opencode 不输出 `## 改动摘要` | prompt 显式强约束 + `extract_report` 全文兜底；审核页本看 diff，不只看报告 |
| auth 探测差异（codex 无干净 status） | claude=`auth status`；opencode=`auth list` 解析；codex 近似用 `--version` 存在 + 配置/login 痕迹，P2 实测微调 |
| model 命名不同（opencode 要 `provider/model`） | `kind` 决定是否校验格式 + 配置占位提示；不强校验，交给 CLI 报错 |
| opencode 起本地 server 略慢 | 沿用 1800s 超时；observe 后 per-kind 可调 |
| 新 agent 漏装安全护栏 | 安全不变量集中在 §4.3 调用层，新 kind 默认继承；评审 checklist 卡 remote-git 测试 |

## 7. 实施阶段

| 阶段 | 内容 | 可验证点 |
|---|---|---|
| **P0** 纯重构 | 抽 `CodeAgent` trait，claude 包成 `CliCodeAgent{kind:claude}`，两调用点走 `resolve()` | 行为零变化，build + 流程通 |
| **P1** 配置链路 | 迁移 0057 + `commands/code_agents.rs` + Settings 区块 | 设置页可管理/切默认，claude 仍正常 |
| **P2** 接 codex + opencode | 补两 kind flag 映射 + 泛化 auth + 通用 git env 护栏 | 各跑通真实 CR，安全四项验证 |
| **P3** per-project 覆盖 | `projects.code_agent_id` + Projects 下拉 + merge.rs 解冲突也走 resolve | 两项目不同 agent 并行 |

## 8. 演进（暂不做）

- **非 CLI 式 code agent**：若要支持「LLM + 自研工具循环直接改 worktree」，需把 trait 抽象从「进程」上提为
  「在 worktree 产生改动」的更高层接口。当前三种均为 CLI，先保 trait 薄而稳；有需求再加。
- **MCP 接入**：code agent 选择与 MCP 工具生态正交，按 CLAUDE.md 愿景后续在后端落地，复用同一 Tool trait。
