# AutoForge 缺失模块盘点 × Innate 核心整合 — 实施文档 v1

> 状态：实施基线（供后续 AI 编码实施）
> 编写日期：2026-06-14
> 关联设计：`.autoforge/docs/设计文档/autoforge-design.md`（v0.9，权威设计基线）
> 代码基线：`src-tauri/`（Tauri 2.11.2 + Rust + sqlx/SQLite），迁移 `0001`–`0018`
> Innate 基线：`/home/renmk/projects/Innate/core`（`KnowledgeBase` Rust lib，8 Public API）

---

## 0. 本文目的与读法

本文档做两件事,供后续 AI **直接照着编码**:

1. **盘点 AutoForge 当前缺失/半成品的功能模块**——对照设计文档 v0.9 与实际代码,逐项给出「现状 / 目标 / 要建什么 / 涉及文件 / 数据模型 / 验收标准」。
2. **把 Innate 作为"自成长"核心模块整合进 AutoForge**——给出精确接入点(文件:符号)、`KbManager` 设计、recall/record/evolve/promote 的数据流、新增命令与迁移。

**实施纪律(来自项目约定)**:
- 前端改动**先读 `DESIGN.md`**,只用 `src/index.css` 的 CSS 变量与既有类,禁止硬编码色值/字号。
- 涉及 Tauri 一律 **2.x API**(`tauri 2.11.2`)。
- 改 AutoForge 自身数据模型**必须**新增 `src-tauri/migrations/*.sql` 并由 `sqlx::migrate!` 执行(见 design §9.5)。
- 所有 ID 用 UUID 字符串;时间用 `datetime('now')` 文本;数组/对象以 JSON 存 `TEXT`。

---

## 1. 现状基线:已建 / 部分 / 缺失

对照设计 §7 八节点全链路 + 横切能力,基于实际源码盘点:

### 1.1 八节点交付链路

| 节点 | 设计 | 实现状态 | 代码证据 |
|---|---|---|---|
| 01 需求收集 | §6 双轨入口 | ✅ **已建** | `intake/{gateway,github,scanner,webhook,bulk}.rs`;命令 `submit_issue / sync_github_issues / run_code_scan / bulk_import_issues` |
| 02 物料管理 | 官网节点 | ✅ **已建(较完整)** | `commands/materials.rs`(CRUD + `ai_organize_materials` + 备份);迁移 `0014` |
| 03 原型设计 | 官网节点/design 未深设 | ❌ **缺失** | 无后端模块、无命令、无表 |
| 04 Spec 审查 | §9 规范体系 | 🟡 **半成品** | `commands/specs.rs`(CRUD + `ai_generate_specs`);但 §9.3「规范盲区→管理员→沉淀规范」反馈闭环**未建** |
| 05 编码实现 | §7 / §10.2 | ✅ **已建** | `tasks/execution.rs`(worktree + 预览记录 + 迭代软上限);`agents/{code_agent,local_claude}.rs`(claude CLI 调用 + 报告抽取) |
| 06 系统测试 | §6.2 / §10.3 | 🟡 **半成品** | `tasks/testing.rs` 仅按 `autoforge.yaml` 跑 shell 检查并建 Bug issue;**不是**会"发现遗漏、区分误报、主动巡检"的测试 Agent |
| 07 安全审查 | §4.3 三层防护 | ❌ **基本缺失** | `core/security.rs` 仅 34 行(正则注入匹配 + 指纹);**Layer 1 LLM 消毒、Layer 2 行为审计 Agent 均未建**;无代码安全审查节点 |
| 08 部署上线 | 官网节点 | ❌ **缺失** | 无部署模块、无 Shell 脚本生成、无 main 合并流程 |

### 1.2 横切能力

| 能力 | 设计 | 状态 | 证据 |
|---|---|---|---|
| 任务队列/幂等 | §13 | ✅ | `tasks/runner.rs`(`tokio::mpsc` + `job_executions` + `INSERT OR IGNORE`) |
| 背压三阶段 | §7.1 | ✅ | `runner.rs::wait_for_execution_slot`(full/paused 判定) |
| 并发槽位 | §11 | ✅ | `core/concurrency.rs::ConcurrencyManager` |
| 迭代软上限 | §10.4 | ✅ | `execution.rs:51` `ITERATION_SOFT_LIMIT=3` + `IterationWarning` 事件 |
| 双人类门 | §7 | ✅ | `commands/change_requests.rs::review_1 / review_2` + `admin_decisions` 审计 |
| 实时事件 | §8.2 | ✅ | `core/event.rs` Tauri `autoforge://event` |
| 预览系统 | §5 | 🟡 仅 `file://` worktree URL | `execution.rs:142-155`;容器/快照/脱敏(M5)全未建,`preview_environments` 已留字段 |
| 通知 Hub | §13 | 🟡 仅 Tauri 事件 | 无邮件/Slack/企微 |
| 嵌入式 Widget | §6.1 | ❌ 缺失 | M10 |
| 多项目隔离 | §11 | 🟡 部分 | `projects` 表 + 按 `project_id` 查询;并发为全局+按项目 |
| **自进化/成长框架** | §1.4 | ❌ **未建** | **本文档第 4 章 = 用 Innate 落地它** |

### 1.3 一句话现状

> **AutoForge 的"自动化骨架"已经立住**(需求→分析→双门→Claude 执行→测试→合并的闭环能跑),**但"自进化的大脑"完全是空的**,且链路两端(原型 03 / 安全 07 / 部署 08)和质量纵深(智能测试、预览 M5)是缺口。Innate 整合补的正是"自进化大脑"这一最关键缺失。

---

## 2. 缺失功能模块详细盘点(非 Innate 部分)

> 每个模块给出可直接开工的规格。Innate 相关的"自成长"整合单列第 4 章。

### 2.1 安全纵深(最高优先 — 设计 §4.3 明示"安全是前提不是功能")

当前 `core/security.rs` 只有正则关键词匹配(`has_obvious_injection`)。设计要求**三层防护**,缺口:

**Layer 1 — 输入消毒(LLM 级)**
- 现状:无。所有外部输入(Widget/GitHub/API)进入分析 Agent 前**未经 LLM 安全检测**。
- 要建:`intake/sanitize.rs` —— 在 `intake/gateway.rs` 入队前,对每条外部输入调一次轻量 LLM(可复用 `agents/llm.rs`,用 haiku/小模型)分类「正常 / 含注入指令 / 含敏感个人信息」。命中即丢弃 + 写安全日志,不进流水线。
- 数据模型:新增迁移 `security_events(id, source_type, raw_excerpt, verdict, model_id, created_at)`。
- 接入点:`intake/gateway.rs` 的入口函数,在写 `issues` 前调用。
- 验收:构造含 "ignore previous instructions" 的 GitHub issue → 被拦截入 `security_events`,不产生 `issues` 行。

**Layer 2 — 行为审计 Agent**
- 现状:无。`core/git.rs::GitProxy` 仅拦已知危险 git 命令(Layer 3 雏形)。
- 要建:`tasks/execution.rs` 执行 claude 期间,捕获其 git/文件操作流,交独立安全 Agent 实时判定;异常(写非 `autoforge/*` 分支等)立即终止会话。一期可先做**操作日志全留 + 规则判定**,LLM 审计二期。
- 数据模型:`audit_events(id, worktree_session_id, op_type, op_detail, verdict, created_at)`。

**Layer 3 — 分支操作双重确认**
- 现状:部分。`GitProxy` 存在,但合并由 `tasks/merge.rs` 在 review_2 后自动发起;**main 分支**应禁止自动改(design §4.1),需确认 merge 只入 dev。
- 要建:校验 merge 目标恒为 `branch_dev`;任何 main 操作必须独立人工确认命令。

**代码安全审查节点(07)**
- 要建:`tasks/security_audit.rs` —— 合并前/后对 diff 跑安全扫描 Agent(SAST 思路:密钥泄露、注入、危险依赖),产出 findings 入 `scan_findings`,高危自动建 issue。可复用 `testing.rs` 的 `scan_findings` 通路。

### 2.2 测试 Agent 智能化(节点 06,设计 §10.3)

- 现状:`tasks/testing.rs` = 读 `autoforge.yaml` 跑 unit/integration/lint/typing/security 命令,失败建 Bug issue。是**配置驱动 runner**,非 Agent。
- 缺口:(a) 发现 Claude 遗漏的问题;(b) 区分真实 Bug 与环境误报;(c) **主动巡检模式 B**(每日全量 + 质量周报,design §6.2)完全未建,`session_type='proactive'` 字段已留但无调度。
- 要建:
  1. `tasks/testing.rs` 增加 LLM 评估层:把 shell 检查结果 + diff 交测试 Agent 判定严重级别与误报,降低噪声入队。
  2. `tasks/scan.rs` + 定时调度(`tokio` interval 或 cron):主动巡检全量套件 + 质量周报落 `test_sessions(session_type='proactive')`。
- 验收:被动响应已可用;新增主动巡检每日产出一份 `proactive` test_session。

### 2.3 原型设计引擎(节点 03)

- 现状:无。官网定位为"生成可直接用于 OpenDesign/Stitch/Claude Design 的设计提示词"。
- 要建(轻量):`commands/prototype.rs` —— 基于 issue/spec/materials 生成结构化**设计提示词**(LLM),存为一种 material 或独立表 `prototype_prompts(id, project_id, issue_id, prompt, tool_target, created_at)`,前端可一键复制。**不做**内嵌设计器,只做提示词产出 + 外链。
- 优先级:低(链路价值小于安全/部署)。

### 2.4 部署上线(节点 08)

- 现状:无。`merge.rs` 只合到 dev。
- 要建:`tasks/deploy.rs` —— 基于 `autoforge.yaml` 的 build/start 声明,LLM 生成稳定 Shell 部署脚本 → 人工确认 → 执行到目标环境;落 `deployments(id, project_id, cr_id, script, target_env, status, log, created_at)`。
- 安全:部署脚本执行属高危,**必须**第三个人工确认门(复用 Layer 3 思路)。
- 优先级:中(端到端闭环的最后一段)。

### 2.5 预览系统 M5(设计 §5)

- 现状:仅记录 `file://{worktree_path}`。
- 缺口:容器化预览、预览数据库快照、字段级脱敏规则引擎(`preview.sensitive_fields`)、热重载、路径路由。
- 要建:按 design §5.2/§5.3 的 M5 范围,优先 **seed 脚本路径** + 脱敏引擎(`mask/hash/drop`),容器化用 Podman。`preview_environments` 的 `data_masked_at / mask_policy_version` 字段已留。
- 优先级:中(影响审核体验,不阻塞自成长闭环)。

### 2.6 规范文档反馈闭环(设计 §9.3)

- 现状:`specs.rs` 有 CRUD,但「Agent 标注规范盲区 → 管理员决策 → 沉淀为规范更新」的回路**未建**。
- 要建:这正是 **Innate 的天然落点**——见第 4 章,把规范盲区与门决策喂给 Innate,蒸馏后回流规范/shared 知识。

### 2.7 通知 Hub / Widget(M10)

- 通知:现仅 Tauri 事件,缺邮件/Slack/企微外部通道 → `core/notify.rs` 适配器。
- Widget:`autoforge-widget` 纯 JS SDK + 隐私策略(截图客户端脱敏、IP 哈希、180 天保留)。低优先。

---

## 3. Innate 整合总览:为什么、怎么接

### 3.1 定位

AutoForge 设计 §1.4「自进化运行模式」要求工厂**通过运行积累经验、持续改进**,但当前**没有任何记忆机制**——每个需求孤立处理,门决策只写 `admin_decisions` 审计表后即沉睡,不反哺未来。

**Innate 就是这块缺失的"程序性记忆 + 自成长引擎"。** 整合后:
- 每次 Claude 执行前,recall 注入**历史同类经验 + 跨项目通用技能**;
- 每个人类门决策(review_1/review_2)、每次测试结果,record 为训练信号;
- 需求收尾 evolve 蒸馏成知识;
- 跨项目通用经验 promote 到共享层 —— **shared 层就是"成长框架"积累手艺的物理载体**。

### 3.2 集成方式:Rust lib 直连(不走 MCP 子进程)

AutoForge 后端是 Rust,Innate core 是 Rust lib。两者同语言 → **把 `innate` 作为 path 依赖直接 in-process 调用**,与 Innate 自身 MCP/CLI/Web 接入模块同构(它们都直连 `KnowledgeBase`)。

`src-tauri/Cargo.toml`:
```toml
[dependencies]
innate = { path = "../../Innate/core" }   # 复用 KnowledgeBase lib
```
> 注:Innate crate 名以其 `core/Cargo.toml` 的 `[package].name` 为准(`lib.rs` 暴露 `KnowledgeBase`、`open_with`)。若包名非 `innate`,按实际改。也可先 vendored 或 git 依赖,避免相对路径耦合。

**不要**让 AutoForge 起 `innate mcp` 子进程——同进程直调零序列化、事务可控、可注入自定义 `Distiller/Sanitizer`,且写入能与现有 `tokio` runtime / 并发控制协调。

### 3.3 租户模型(已决策):每项目一 db + 共享层

```
~/.autoforge/kb/
  shared.db          ← 跨项目通用程序性知识(技能 + 稳定性负面知识)= 成长框架的记忆
  proj-<project_id>.db   ← 每个 AutoForge 项目独立(工作记忆,可随项目归档)

recall(project_id, q) = merge( proj_kb.recall(q), shared_kb.recall(q) )
record / evolve        → 默认写项目库
promote                → 项目知识泛化后晋升到 shared
```

**为什么**:不同项目独立 SQLite 文件 → 天然分摊写锁(Innate `record()` 是 `BEGIN IMMEDIATE` 单写者,与 AutoForge 现有 `ConcurrencyManager` 不冲突);shared 是工厂护城河,承载越用越强的手艺。

**硬约束**:所有库(每个 proj + shared)**必须用同一 embedding 模型与维度**,否则向量不可比、chunk 无法在库间搬动、fan-out 失效。在 `KbManager` 钉死一份 embedding config 注入每个 `open_with`。

---

## 4. Innate 整合实施细节

### 4.1 `KbManager`(新增:`src-tauri/src/knowledge/mod.rs`)

工厂侧唯一的知识编排层。Innate Core 一行不改——所有项目特异性收在这里。

```rust
// src-tauri/src/knowledge/mod.rs  (新增模块)
pub struct KbManager {
    shared: Arc<KnowledgeBase>,                          // shared.db,常驻
    projects: Mutex<LruCache<String, Arc<KnowledgeBase>>>, // proj-<id>.db,LRU 懒开
    embedding: EmbeddingConfig,                          // 全库统一,钉死
    kb_root: PathBuf,                                    // ~/.autoforge/kb/
}

impl KbManager {
    pub fn open(kb_root: PathBuf, embedding: EmbeddingConfig) -> Result<Self>;
    fn project_kb(&self, project_id: &str) -> Arc<KnowledgeBase>; // 懒开 + LRU
    fn shared_kb(&self) -> Arc<KnowledgeBase>;

    // 核心三方法 —— 见 4.3
    pub fn recall(&self, project_id: &str, query: &str, k: usize) -> Vec<RecallHit>;
    pub fn record(&self, project_id: &str, args: RecordArgs) -> Result<()>;
    pub fn evolve_project(&self, project_id: &str) -> Result<EvolveReport>;
    pub fn promote_to_shared(&self, project_id: &str, chunk_id: &str) -> Result<()>;
}
```

**要点**:
- `KnowledgeBase::open` 会把该库全部 embedding 载入内存 → **不能把所有项目库常开**,用 LRU(如 `lru` crate,容量 8)+ 空闲关闭。
- 挂到 `AppState`(`src-tauri/src/state.rs`)新增字段 `pub kb: Arc<KbManager>`,在 `lib.rs` 启动时与 `db`/`concurrency` 一起初始化。
- embedding/LLM 配置复用 AutoForge 现有 `llm_configs` 或新增 `app_settings` 键。

### 4.2 fan-out recall 合并(`KbManager::recall`)

```
hits_proj   = proj_kb.recall(query, k)
hits_shared = shared_kb.recall(query, k)
return interleave(hits_proj, hits_shared, 去重, 截断 k)
```
⚠️ 两库融合分(含库内置信度/使用统计)**严格不可比**。v1 用「项目库优先 + shared 兜底降权」的交错,**不假装精确排序**。shared 命中的条目应带"通用技能"标记,供 prompt 注入时分区展示。

### 4.3 精确接入点(文件:符号 — 直接照改)

| # | 接入点 | 文件:符号 | Innate 调用 | 写哪个库 |
|---|---|---|---|---|
| ① recall→分析 | 分析 Agent 起手 | `tasks/analysis.rs::run` / `agents/analysis.rs` | `kb.recall(project_id, issue.title+desc)` 注入 prompt | 读 proj⊕shared |
| ② recall→编码 | **构建 Claude prompt 前** | `tasks/execution.rs:116` `code_agent::build_prompt(...)` | `kb.recall(project_id, issue+analysis)` 结果作为新参数拼入 prompt | 读 proj⊕shared |
| ③ record→门1 | 审核1 决策 | `commands/change_requests.rs::review_1`(:179 approved / :227 rejected) | `kb.record(project_id, {outcome, kind:"需求判断", content: issue+decision+suggestions})` | proj |
| ④ record→门2 | 审核2 决策 | `commands/change_requests.rs::review_2`(:266 approved / :310 rejected / :363 revision) | `kb.record(... kind:"代码程序性", content: diff摘要+decision+suggestions)` | proj |
| ⑤ record→测试 | 测试结果 | `tasks/testing.rs::run`(:80 status) | `kb.record(... outcome: passed?ok:fail)` | proj |
| ⑥ evolve | 需求收尾(merge 完成/rejected) | `tasks/merge.rs::run` 末尾 / review_2 rejected 分支 | `kb.evolve_project(project_id)` | proj |
| ⑦ promote | 跨项目复现达阈值 | 定时任务 / evolve 后钩子 | `kb.promote_to_shared(project_id, chunk_id)` | proj→shared |

**最省力的金信号来源**:`change_requests.rs::record_admin_decision`(:424)已经把每个门决策写入 `admin_decisions`。**在该函数内追加一行 `kb.record(...)`**,即可零散点地把全部人类门决策镜像进 Innate——这是整合的最高杠杆切入点。

**②的具体改法**:`build_prompt` 当前签名(execution.rs:116-126)接收 issue/analysis/suggestions/config。新增一个 `recalled_knowledge: &str` 参数,把 `kb.recall()` 的命中(shared 技能 + proj 经验,分区标注)拼进 prompt 的上下文段。Claude 执行结束后,在 worktree_session 完成处(execution.rs:203 附近)按 exit_code 调一次 `kb.record(outcome)` 闭合 trace。

### 4.4 promote = 泛化 + 治理(自成长的发动机)

shared 是工厂护城河,**promote 不是复制而是泛化**:
1. **触发信号**(比"复用N次"硬):同一程序性模式在 ≥K 个不同 `project_id` 库被独立蒸馏出 + 置信度 EMA 持续高 + 来自人类门(>agent 自报)。
2. **泛化步骤**:proj chunk 是具体的("项目X的auth用Y"),进 shared 前过一道 LLM 蒸馏剥离项目专名 → 可迁移技能("这类auth倾向需要Y"),并写强 **trigger 描述(适用边界)**(Innate `tvec` 即为此设计),防"一招鲜套所有项目"的过拟合。
3. **shared 治理**(它错了坑所有未来项目,需比 proj 严):
   - 更高准入门槛,值得设**第三个人类门**"是否固化为工厂技能"(低频高杠杆,符合 Human-Lite);
   - 语义冲突检测(两条 shared 知识矛盾上报);
   - 衰减(久不复用降权,防基因组膨胀)。

> v1 可先把 promote 做成**人工触发 + 简单阈值**,泛化蒸馏与冲突检测二期。但架构上 promote/shared 是一等公民,不是边角。

### 4.5 新增 Tauri 命令(`commands/knowledge.rs`)

供 Review Portal 展示与人工治理:
```
kb_recall_preview(project_id, query)     -> 调试:看会注入什么
kb_list_shared_skills()                  -> shared 技能列表(治理界面)
kb_promote(project_id, chunk_id)         -> 人工晋升到 shared(第三门)
kb_inspect()                             -> 知识库健康度(复用 Innate inspect)
kb_stats(project_id)                     -> 该项目知识条数/置信度趋势
```
在 `lib.rs::invoke_handler!` 注册;前端新增「知识库/Knowledge」页(遵守 `DESIGN.md` ember 风格,复用 `.panel/.stat/.chip`)。

### 4.6 数据模型与迁移

Innate 知识存于 `~/.autoforge/kb/*.db`(Innate 自管 schema,**不进 AutoForge 迁移**)。AutoForge 侧只需少量关联表(走 `src-tauri/migrations/0019_knowledge.sql`):
```sql
-- 把 AutoForge 实体关联到 Innate trace/chunk,便于审计与回链
CREATE TABLE knowledge_links (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  ref_type TEXT NOT NULL,      -- issue | change_request | admin_decision | test_session
  ref_id TEXT NOT NULL,
  innate_trace_id TEXT,        -- recall 返回的 trace_id
  innate_chunk_id TEXT,        -- record/promote 产生的 chunk
  kind TEXT,                   -- requirement_judgment | code_procedural | stability
  created_at TEXT DEFAULT (datetime('now'))
);
```

### 4.7 自成长闭环 = 设计 §1.4「成长框架」的落地

```
需求进入 ──► ① recall(proj⊕shared) 注入分析/编码 prompt
   │
   ├─ 审核1 批准/否决 ──► ③ record(需求判断) ──┐
   ├─ 审核2 改/退/合并 ──► ④ record(代码程序性) ─┼─► ⑥ evolve(蒸馏→curate) 写 proj
   └─ 测试 通过/失败 ──► ⑤ record(task_ok/fail) ─┘
                                                      │
                          跨项目复现 + 高置信 ──► ⑦ promote(泛化+治理) ──► shared
                                                      │
                          下一个项目/需求 ──► ① recall 命中 shared 技能(开箱即用)
```
**人按一次批准键 = 工厂厚一分记忆;shared 厚到一定程度反向驱动门控降级**(某类变更历史从不被改 → 审核2 可建议自动过),Human-Lite 趋向 Human-Optional —— 这就是 §1.4 自进化的字面实现。

---

## 5. 分阶段实施路线

> 依赖顺序排列;每阶段可独立验收。Innate 整合(P1)优先,因为它是设计核心缺失且解锁后续飞轮。

| 阶段 | 内容 | 关键交付 | 验收 |
|---|---|---|---|
| **P0 安全前提** | Layer 1 LLM 输入消毒 + Layer 3 main 保护校验 | `intake/sanitize.rs`、`security_events` 表 | 注入样本被拦截不入 `issues` |
| **P1 Innate 接入骨架** | `KbManager`(open/recall/record) + path 依赖 + AppState 挂载 | `knowledge/mod.rs`、`0019_knowledge.sql` | 一个需求跑通:recall 注入 prompt + 门决策 record 成功 |
| **P2 金信号全量接入** | `record_admin_decision` 内镜像 record + 测试结果 record + evolve 收尾 | 接入点 ③④⑤⑥ | `admin_decisions` 每条都有对应 Innate chunk;evolve 产出 pending chunk |
| **P3 shared 与 promote** | fan-out recall 合并 + promote(人工+阈值) + 知识页 + `kb_*` 命令 | 接入点 ⑦、`commands/knowledge.rs`、前端 Knowledge 页 | 跨项目命中 shared 技能;管理员可人工晋升 |
| **P3.5 Diff 分级降档** | Grader(T0-T3)+ 信任状态机 + 审核页分级 + 一键/超时;真自动过待 shared 成熟 | `agents/grader.rs`、`0021_grading.sql`、`Audit.tsx` | 每 CR 带 tier;T3 永远人工;某类达阈值转 eligible;抽检失败→降级(§7) |
| **P4 测试 Agent 智能化** | LLM 评估层 + 主动巡检 + 质量周报 | `tasks/scan.rs` + 调度 | 每日产出 `proactive` test_session |
| **P5 安全审查 + 行为审计** | Layer 2 行为审计 Agent + 代码安全审查节点(07) | `tasks/security_audit.rs`、`audit_events` | 高危 diff 被扫描建 issue |
| **P6 部署上线(08)** | Shell 脚本生成 + 第三确认门 + 执行 | `tasks/deploy.rs`、`deployments` 表 | 一键部署到目标环境(人工确认后) |
| **P7 预览 M5** | seed 路径 + 脱敏引擎 + 容器化 | 按 design §5 | worktree 预览可点击真实运行 |
| **P8 原型/通知/Widget** | 设计提示词 + 外部通知 + Widget SDK | 低优先补全 | — |

**关键路径**:P0 → P1 → P2 → P3 是自成长引擎主线,应连续推进;P4–P8 是链路补全,可按业务优先级穿插。

---

## 6. 风险与约束

| 风险 | 说明 | 对策 |
|---|---|---|
| embedding 不一致 | proj/shared 用不同模型 → 向量不可比、promote 失效 | `KbManager` 钉死单一 embedding config,启动校验 |
| 写锁争用 | Innate `record()` 是 `BEGIN IMMEDIATE` 单写者 | 每项目独立 db 天然分摊;shared 写(promote)低频;必要时经 AutoForge 串行通道 |
| 内存膨胀 | 每个 KB open 全量载入 embedding | LRU 限活跃库数(默认 8)+ 空闲关闭 |
| shared 污染 | 错误知识坑所有未来项目 | promote 高门槛 + 第三人类门 + 冲突检测 + 衰减 |
| 过拟合(monoculture) | shared 技能被无条件套用 | 每条 shared chunk 写强 trigger 适用边界(tvec) |
| 安全滞后 | Layer 1 未建即接外部输入 = 高危 | P0 先行,设计 §4.3 明示"安全是前提" |
| Innate 依赖耦合 | path 依赖跨仓 | 评估 vendored/git 依赖;锁定 Innate 版本 |
| 双写一致性 | `admin_decisions` 与 Innate chunk 可能不同步 | `knowledge_links` 关联 + record 失败不阻断主流程(降级记日志) |

---

## 7. 代码 Diff 风险分级与门控降级(Gate Downgrade)

> 目标:**不是所有代码都需要人工审核**。按 diff 风险分级,低风险走轻量/自动通道,高风险强制人工。
> 这是 AutoForge 从 Human-Lite 走向 Human-Optional 的核心机制,**由 Innate shared 层的历史信号驱动其安全性**。
> 策略(已决策):**分阶演进——信任挣来的**。冷启动全人工/一键,某类积累足够正样本后逐类解锁自动过。

### 7.1 四档风险模型

执行完成后(`tasks/execution.rs` 置 `pending_review_2` 之前)插入 **Grader**,给 diff 打档:

| 档 | 含义 | 典型 diff | 冷启动走向 | 成熟走向(该类信任达标后) |
|---|---|---|---|---|
| **T0** | 零风险 | 文档/注释/格式化/纯文案/纯新增测试 | 一键批量确认 | **零人工自动合并 dev** |
| **T1** | 低风险 | 局部小逻辑、测试全过、未碰敏感路径 | 超时未反对则通过 | 一键/自动 |
| **T2** | 中风险 | 常规业务逻辑改动 | 人工 review_2(现行) | 人工 review_2 |
| **T3** | 高/致命 | schema/迁移、auth/支付/安全、大爆炸半径、低覆盖、临近 `forbidden_paths` | **强制人工,置顶高亮** | **强制人工(永不自动)** |

> 自动合并目标恒为 `branch_dev`;**main 永远只人工**(design §4.1)。

### 7.2 分级信号(三类融合,非单靠 LLM)

1. **静态/结构**(便宜确定):改动文件数/行数、命中路径(对照 `autoforge.yaml` `forbidden_paths`/`quality.security`)、是否含迁移/依赖/密钥模式。
2. **流水线**:`testing.rs` 结果(全过?覆盖率 delta)、Claude 自报"潜在风险"段、迭代轮次(≥3 轮天然降到 T2+)。
3. **Innate 学习信号(关键)**:Grader 内 `kb.recall(project_id, diff签名)` 取**历史同类改动的人类门结局**;该类"≥N 次批准 + 0 退改 + 抽检零问题"才有资格降到 T0/T1。把 `admin_decisions` 历史变成可执行的信任。

> LLM 分级 Agent 只做语义裁决,且**只能往严调不能往松调**(防注入诱导降档)。

### 7.3 每"变更类"的信任状态机(分阶演进的核心)

```
冷启动(cold)        该类无足够历史
   │  人工决策累计:≥20 次批准 且 0 退改/拒绝 且 抽检零问题
   ▼
合格(eligible)      Innate 置信度达阈值 → 解锁"一键/超时"
   │  持续稳定(再积累 + 抽检持续零问题)
   ▼
自动(auto)          该类 T0 零人工自动合并、T1 自动
   │  任一自动合并被回滚 / 抽检发现问题 → kb.record(fail)
   ▼
降级(demoted)       立即收紧回 T2,信任清零重挣
```
"变更类"按 diff 签名聚类(路径前缀 + 改动类型 + 模块),粒度由 Grader 定义。

### 7.4 安全护栏(命门,缺一不可)

- **硬地板**:迁移、auth/支付/安全、依赖、配置、`forbidden_paths` 相邻 —— 永远 ≥T2/T3,`autoforge.yaml` 声明,**学习信号无权覆盖**。
- **冷启动保守**:出厂自动过全关,全部走人工;逐类挣信任后才解锁。
- **事后抽检 + 自动收紧**:T0/T1 自动/超时通过的改动按比例抽样补检;发现问题或回滚 → `kb.record(fail)` → 该类降级。
- **测试必跑**:即便 T0,合并后 `testing.rs` 照跑,失败建 Bug issue 入队。
- **可逆 + kill switch**:自动合并 commit 易回滚;管理员一键全局停用自动过。
- **只进 dev,绝不碰 main**。

### 7.5 与背压的协同

design §7.1 背压根因是人工审核跟不上 Claude 产出。分级把 T0/T1 摘出审核队列 → **直接缩小积压基数**,自动过越准、背压越少触发。两机制天然互补。

### 7.6 代码落点

| 改动 | 位置 |
|---|---|
| 新增 Grader | `src-tauri/src/agents/grader.rs` —— 输入 diff(`get_code_diff` 逻辑)+ 测试结果 + `kb.recall`,输出 `{tier, score, rationale, change_class}` |
| 决策分叉 | `tasks/execution.rs:236` 附近——置 `pending_review_2` 前调 Grader:`auto`→`pending_merge`+enqueue merge+发 `AutoMerged` 事件;`eligible`→`pending_review_2` 带 tier(支持一键/超时);否则→现行 `pending_review_2` |
| 超时通过 | 调度器对 `eligible` 档的 CR 设 TTL,到期无人反对自动转 `pending_merge`(可配置) |
| 学习信号 | Grader 内 `kb.recall`;`change_requests.rs::record_admin_decision:424` 已镜像门决策进 Innate,天然喂养信任状态机 |
| 抽检收紧 | `tasks/scan.rs`(P4 主动巡检)抽样 auto/eligible 合并,异常 → `kb.record(fail)` + 降级 |
| 数据模型 | 迁移新增 `change_requests.risk_tier / risk_score / risk_rationale / change_class / auto_decision`;新表 `auto_pass_policy(change_class, trust_state, approve_count, reject_count, updated_at)` |
| 审核页 | `Audit.tsx` 按 tier 分组排序,T3 置顶高亮,T0/T1 支持批量一键 + 显示信任状态(遵守 DESIGN.md) |
| 配置 | `autoforge.yaml` 新增 `review.auto_pass`:`enabled`、各档阈值、硬地板路径黑名单、`eligible` 超时 |

```yaml
# autoforge.yaml 新增段
review:
  auto_pass:
    enabled: false            # 出厂关闭,挣到信任再开
    promote_threshold: 20     # 某类积累多少次"批准且0退改"后解锁
    eligible_timeout_min: 60  # eligible 档无人反对则通过的等待时长
    spot_check_ratio: 0.2     # 自动/超时通过的抽检比例
    hard_floor_paths:         # 永不自动过(学习信号无权覆盖)
      - "migrations/"
      - "**/auth/**"
      - "**/payment/**"
      - ".env*"
```

### 7.7 里程碑placement

作为 **P3.5**(P3 shared 与 promote 之后):
- 真正的 `auto` 档依赖 shared 历史信号成熟 → 必须在 Innate 金信号全量接入(P2)+ shared(P3)之后;
- 但**冷启动的分级排序 + 一键/超时**不依赖 Innate 成熟,可在 P2 后即上线(Grader 的静态+流水线信号先用,Innate 信号随数据增长自动增强)。

**验收**:(1) 每个 CR 带 `risk_tier`,审核页按档排序;(2) T3 永远人工;(3) 某测试类积累阈值样本后自动转 `eligible`,UI 显示信任状态;(4) 注入抽检失败样本 → 该类降级回 T2。

---

## 8. 附录:文件改动速查表

**新增**
```
src-tauri/src/knowledge/mod.rs          KbManager(open/recall/record/evolve/promote)
src-tauri/src/commands/knowledge.rs     kb_* Tauri 命令
src-tauri/src/intake/sanitize.rs        Layer 1 LLM 输入消毒
src-tauri/src/tasks/security_audit.rs   节点07 代码安全审查
src-tauri/src/tasks/scan.rs             主动巡检 + 质量周报
src-tauri/src/tasks/deploy.rs           节点08 部署
src-tauri/src/core/notify.rs            外部通知适配器
src-tauri/src/commands/prototype.rs     节点03 设计提示词(低优先)
src-tauri/src/agents/grader.rs          diff 风险分级器(T0-T3 + 信任状态机,§7)
src-tauri/migrations/0019_knowledge.sql knowledge_links
src-tauri/migrations/0020_security.sql  security_events / audit_events
src-tauri/migrations/0021_grading.sql   change_requests 风险字段 + auto_pass_policy
src/pages/Knowledge.tsx                 知识库治理页(遵守 DESIGN.md)
```

**修改**
```
src-tauri/Cargo.toml                     + innate path 依赖 + lru
src-tauri/src/state.rs                   AppState + kb: Arc<KbManager>
src-tauri/src/lib.rs                     启动初始化 kb + 注册 kb_* 命令
src-tauri/src/tasks/execution.rs:116     build_prompt 注入 recall 结果;:203 后 record(outcome)
src-tauri/src/agents/code_agent.rs       build_prompt 增加 recalled_knowledge 参数
src-tauri/src/tasks/analysis.rs          分析前 recall 注入
src-tauri/src/commands/change_requests.rs:424  record_admin_decision 内镜像 kb.record(最高杠杆)
src-tauri/src/tasks/testing.rs:80        测试结果 record;增加 LLM 评估层
src-tauri/src/tasks/merge.rs             末尾 evolve_project
src-tauri/src/tasks/execution.rs:236     置 pending_review_2 前调 Grader 决策分叉(§7.6)
src-tauri/src/pages/../Audit.tsx         按 risk_tier 分组排序 + T3 置顶 + T0/T1 批量一键
src-tauri/src/intake/gateway.rs          入队前调 sanitize(Layer 1)
src-tauri/src/core/security.rs           保留正则作快速预筛,LLM 消毒移至 sanitize.rs
```

---

*本文档为实施基线。Innate 整合以其 `core` lib 的 `KnowledgeBase` 8 Public API 为准;AutoForge 改动遵守 `CLAUDE.md`(Tauri 2.x、DESIGN.md 风格契约、迁移强制)。实施前按 P0→P1 顺序推进,安全层不得后置。*
