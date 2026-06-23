# Schema 驱动的 Agent（执行标准 + 优化记录 一体化）

> **状态：🟢 主体落地（2026-06-20 开发完成本批 + 深化）**
> 已实现：脚手架 `agents/schema.rs`（`trait StructuredSchema` + `extract_json` + `parse_or_default` +
> **`extract_json_array` + `parse_array_or_empty`（1→N）** + `record`）、统一表 `agent_outputs`（迁移 `0040`）、
> 样板 `analysis`（dual-write）与 `test`（双层）、读取/查询层 `commands/agent_outputs.rs`、前端「环节产出」浏览器。
> **本批新增（2026-06-20）**：① 批量（1→N）解析路径 + 单测；② **triage 升级为 schema 驱动**
> （`TriageParsed` impl `StructuredSchema`，新增 `clarity_score`/`needs_clarification`/`missing_info`/`duplicate_of`，
> 经 `run_system_role_text_traced` 带 trace 落 `agent_outputs(role=triage)`）；③ **proposer 升级**
> （`ProposalItem` + 结构化 `Evidence{file,line,note}`，一次运行落一行 `agent_outputs(role=proposer,target=project)`）；
> ④ **优化循环工具化（字段级体检）**：命令 `agent_output_field_health` + 前端 Trace「schema 体检」tab（字段填充率 + 状态分布）；
> ⑤ 规范样例防漂移单测（triage/proposer）。**cargo test 104 passed，npm run build 通过。**
> 剩余待办：版本 A/B 对比（§5.2）、失败回灌导出（§5.3）、planner/code/grader 等其余角色接入（§4.3/4.4/4.5）、
> `.schema.json` 文件与 struct 的 CI 级一致性校验（§3.5 进阶）。

> 目标：把每个环节 agent 从「自由 prompt → 自由文本」升级为「**版本化 schema** 既约束推理、又结构化沉淀产出」。
> schema 一物三用：**① 执行标准**（强制 agent 覆盖全部分析角度 → 更稳更强）、
> **② 优化信息源**（结构化记录，优化时拿全面信息而非一坨文本）、
> **③ 优化杠杆**（持续打磨 schema 即持续优化 agent）。

---

## 0. 现状盘点（2026-06-20）

| 环节 / 角色 | 现在的产出 | schema 状态 |
|------------|-----------|------------|
| **analysis（需求分析）** | `IssueAnalysisSpec` 强类型 + `issue_analyses` 落库 + dual-write `agent_outputs` + `schemas/issue_analysis.schema.json` | ✅ **完备样板（1→1）** |
| **test（测试）** | `agents/test_agent.rs`（机器权威 verdict/checks + LLM 诊断 coverage/failures）+ `schemas/test_report.schema.json` + 写 `agent_outputs` | ✅ **第 2 样板（1→1，双层）** |
| 脚手架 | `agents/schema.rs`：`StructuredSchema`（`ROLE`/`VERSION`/`schema_template`/`prompt_contract`）、`extract_json`、`parse_or_default`、`record`（10 参，含 32k 截断） | ✅ **已实现（仅单对象）** |
| 读取/查询层 | `commands/agent_outputs.rs`：`list_agent_outputs`(filter)/`get_agent_output`/`list_agent_output_roles`/`clear_agent_outputs` | ✅ **已实现** |
| 前端浏览器 | `Trace.tsx` `AgentOutputsExplorer`：role/target 筛选 + 详情 + 点 `trace_id` 跳「单步推理」tab | ✅ **已实现** |
| planner（调度器） | `ConversationPlan` JSON（编排层），未走 `StructuredSchema`、不落 `agent_outputs` | 🟡 半结构化 |
| **triage（炼噪声）** | `intake/triage.rs`：`run_system_role_text("triage")`，**批量 JSON 数组** ad-hoc 解析，不落 `agent_outputs` | 🔴 **待升级（战略最高）** |
| **proposer（自动供料）** | `intake/proposer.rs`：纯函数返回 `IntakePayload`，**数组** ad-hoc 解析，不落 `agent_outputs` | 🔴 待升级 |
| code/execution、grader、security、doc_writer、spec_writer、summarizer | 自由文本 `PROMPT_*` | 🔴 未做 |

**结论**：范式跑通、脚手架 + 统一表 + 读取层 + 前端浏览器都已就位（比旧版文档 §6「建议」走得更远）。
真正剩下的不是「能不能存」，而是四件具体工程：**(A) 批量产出路径、(B) 把 triage/proposer 接进来、
(C) 防 schema 三表征漂移、(D) 把 §5 优化循环从概念变成可点的工具**。下文逐一给出落地级方案。

---

## 1. 范式拆解（从 `analysis.rs` / `test_agent.rs` 提炼）

一个 schema 驱动 agent = 5 件套（已被脚手架固化）：

1. **schema JSON 模板**：`StructuredSchema::schema_template()` 返回带字段注释的 JSON 片段（内嵌进 prompt 作执行标准）。
2. **强类型 Rust spec**：serde struct，**所有字段 `#[serde(default)]`**（前向兼容：模型漏字段不崩，只降质量）。
3. **prompt 内嵌 schema 作执行标准**：`StructuredSchema::prompt_contract()` 自动渲染「输出契约」块（含版本号 + 模板 + 通用约束），调用方拼到 user/system prompt 末尾。
4. **健壮解析**：`extract_json`（切最外层 `{...}`）→ `parse_or_default::<T>`（类型化反序列化，失败回退 `T::default()` 并返回 `status`），不阻断主流程。
5. **结构化落库 + 版本戳**：`record(db, role, version, target_kind, target_id, project_id, trace_id, status, output_json, raw)` 统一写 `agent_outputs`，best-effort。

**两类拓扑（关键区分，决定落库语义）**：

- **1→1（analysis、test、code、grader…）**：一次环节产出一份报告，`target_id` = 该实体 id（issue/cr）。脚手架现成。
- **1→N（triage、proposer）**：一次运行整理/提议出**多条** issue，需逐条落库（`target_id` = 各 issue.id）。
  现有 `parse_or_default` 不覆盖此形态——见 §2.1 批量扩展。

---

## 2. 脚手架现状与待补的批量扩展

`agents/schema.rs` 已落地（纯 Rust、零 Tauri）。**待补**的是 1→N 形态支持：

### 2.1 批量产出扩展（新增，向后兼容）

在 `agents/schema.rs` 增两个自由函数，不改动既有单对象 API：

```rust
/// 切出最外层 `[...]` 数组并逐元素解析为 T；坏元素跳过，返回 (Vec<T>, status)。
/// status: ok=数组完整解析；partial=部分元素坏；error=非数组/全坏（回退空 Vec）。
pub fn parse_array_or_empty<T: DeserializeOwned>(text: &str) -> (Vec<T>, ParseStatus);

/// 批量落库：对 1→N 的环节，逐条写一行 agent_outputs（共享 role/version/trace_id，
/// 各自 target_id）。返回写入的行 id 列表。
pub async fn record_each(
    db: &Db, role: &str, version: &str, target_kind: &str,
    items: &[(String /*target_id*/, Option<String> /*project_id*/, &str /*status*/, String /*output_json*/)],
    trace_id: Option<&str>, raw: &str,
) -> Vec<String>;
```

要点：
- `parse_array_or_empty` 复用 `extract_json` 的「容忍前后噪声」思路，但锚 `[`/`]`；逐元素 `from_value` 容错。
- triage 现有「批响应漏 idx → 回退单条」的健壮逻辑（`intake/triage.rs:154`）保留，只把**解析**换成类型化、把**结果**落 `agent_outputs`。
- `record_each` 与 §4.5「落库时机」配合：proposer 是纯函数、issue id 尚不存在时**不在 propose() 内落库**，由调用方在 issue 落库后回填。

### 2.2 双层产出范式（test 已示范，code/grader 复用）

`test_agent.rs` 确立了一个值得推广的范式：**机器执行结果是事实权威，LLM 只补分析**。
- `baseline()` 从机器结果（退出码/检查）构造权威字段（`verdict`/`checks`/`summary`）；
- 仅在需要时 `enrich_with_llm()` 补 `coverage`/`failures`/`recommendations`，机器字段不被 LLM 覆盖；
- `status`：机器 OK + LLM 成功=`ok`；机器 OK + LLM 失败=`partial`；解析全失败=`error`。
凡「有客观可执行检查」的环节（code 的编译/测试、security 的扫描器）都按此双层，避免 LLM 篡改事实。

---

## 3. 统一结构化产出表 `agent_outputs`（已实现，迁移 `0040`）

```sql
CREATE TABLE agent_outputs (
  id, role, schema_version, target_kind, target_id, project_id,
  trace_id,              -- 链回 llm_traces（单步推理下钻）
  status,                -- ok | partial | error
  output_json,           -- 完整结构化产出（>32k 截断）
  raw,                   -- 原始模型文本（审计，截断）
  created_at
);
-- 索引：target(kind,id)、role+version、trace_id、project、created_at —— 已建。
```

设计要点（已落实）：
- **additive**，不动 `issue_analyses`/`test_sessions`；analysis dual-write，新 agent 直接只用本表。
- `trace_id` 是与 `llm_traces` 的桥：流水线级（本表）↔ 单调用级（traces），前端 `Trace.tsx` 已打通下钻。

> **粒度规约**：`agent_outputs` = 「一个环节对一个实体的一份结论」（流水线级）。
> 同一实体被同一 role 多次处理（如 review_2 打回重测）会**追加多行**（按 `created_at` 取最新即「现状」，全历史即「演化」）。不做 upsert，保留审计链。

---

## 3.5 Schema 三表征的防漂移治理（新增——当前未强制）

现状有**三处**描述同一 schema，文档曾声称「同源」，实际无强制：
1. **Rust struct**（`TestReport` 等）——落库/解析的真源；
2. **`schema_template()` 内嵌模板**——喂给 LLM 的执行标准；
3. **`agents/schemas/<role>.schema.json`** 文件——对外/审计的独立描述。

三者易漂移（改 struct 忘改模板 → LLM 仍按旧契约产出 → 解析降质量但不报错，最隐蔽）。治理规则：

- **struct 为唯一真源**；`schema_template()` 是它的「prompt 投影」，紧邻 struct 维护。
- **加单元测试卡漂移**（纯 Rust，进 `agents/schema.rs` 的 `#[cfg(test)]` 或各 agent 模块）：
  - `schema_template()` 本身是合法 JSON 骨架（去占位后可 parse）；
  - 用一份「规范样例 JSON」`from_str::<T>()` 成功且关键字段非默认（证明 struct 能吃下模板声明的字段）；
  - `T::VERSION` 与模板里 `"schema_version"` 字面量一致（防版本号忘改）。
- **`.schema.json` 文件**降为「由 struct 生成或校验」的产物（可选：引入 `schemars` 从 struct 派生 JSON Schema，CI 比对；MVP 阶段先靠上面的样例测试兜底，不强依赖 `.schema.json`）。

> 收益：schema 演进时漂移在 `cargo test` 即暴露，而不是上线后靠「字段长期为空」慢慢发现。

---

## 4. 推广顺序与各角色 schema 规格（按战略价值排）

紧扣「脏输入 → 外包上游」主线：**入口炼化 + 流水线闸口**最该先上。下列给到落地级字段。

### 4.1 triage（炼噪声为需求）— 战略最高，1→N

升级 `intake/triage.rs`：把 ad-hoc `triage_from_value` 换成类型化 spec，落 `agent_outputs(role="triage", target_kind="issue", target_id=issue.id)`。

```jsonc
// TriageItem v1.0（批量时为数组元素，单条时为对象）
{
  "idx": 0,                       // 批量模式回指输入序号（落库时换成 issue.id）
  "title": "<精炼后的一句话需求>",
  "category": "<Bug|Feature|Improvement|Chore>",
  "severity": "<low|medium|high|critical>",
  "description": "<结构化补全后的描述>",
  "clarity_score": 0.0,           // 0-1：输入可执行清晰度
  "needs_clarification": false,   // 红线字段：true=应追问而非静默猜（见 dirty-input §3①）
  "missing_info": ["<缺失的关键信息点>"],
  "duplicate_of": null,           // 疑似重复的 issue id 或 null
  "is_noise": false               // 噪音/无价值，直接丢弃（已有语义）
}
```

- **`needs_clarification` + `missing_info` 是外包场景的命脉**：直接度量「静默猜 vs 追问」红线，是 §5 体检与 dogfooding 的对接点。
- 保留现有 batch/denoise/idx-回退逻辑（§2.1），仅替换解析与落库。`denoise_in_place` 与 `refine_triage` 两路都在 issue 已有 id 后落库。
- `status`：数组完整=`ok`，部分元素回退单条=`partial`，全失败=`error`。

### 4.2 proposer（自动供料）— 1→N，落库时机特殊

升级 `intake/proposer.rs`：证据结构化为数组，工程类强制 `file:line`。

```jsonc
// ProposalItem v1.0
{
  "title": "...", "category": "Bug|Improvement|Feature",
  "severity": "low|medium|high",
  "kind": "engineering|feature",     // engineering 必须带 evidence；feature 允许少量、需理由
  "rationale": "<为什么值得做>",
  "evidence": [{"file": "src/...", "line": 42, "note": "<缺重试/硬编码/未覆盖分支>"}],
  "impact": "<影响面>", "effort": "<S|M|L>"
}
```

- **落库时机**：`propose()` 是纯函数、返回 `IntakePayload`，issue 尚不存在。
  → 在调用方（`commands/intake.rs` / `tasks/autosupply.rs`）经 `gateway::receive` 拿到新 issue id 后，
  用 `record_each` 回填 `target_id=issue.id`；被去重/注入过滤丢弃的提议落 `status="error"`（target_kind="proposal", target_id=临时 uuid）或干脆不落，二选一（推荐：丢弃的不落，避免噪声）。
- 安全护栏不变：产出过 `has_obvious_injection`，永远 `IntakeMode::Triage`，绝不自动进流水线。

### 4.3 code / execution 结果 — 1→1，双层

代码实现 agent 跑完后产一份结构化自检，落 `agent_outputs(role="code", target_kind="cr")`，让审核 2 更快：

```jsonc
{
  "summary": "<改了什么 + 为什么>",
  "changed_files": [{"path": "...", "why": "..."}],
  "dod_checklist": [{"item": "<对应 acceptance_criteria>", "met": true, "note": "..."}],
  "self_review_risks": ["<自审风险点>"],
  "out_of_scope_touched": ["<动到 scope 外的文件，应为空>"]
}
```

机器权威部分可取 `git diff --stat`/改动文件清单（事实），LLM 补 `why`/`risks`（双层范式）。

### 4.4 planner（半结构化 → 纳管）

`ConversationPlan` 已是 JSON，成本最低：给它 `impl StructuredSchema`（`ROLE="planner"`），
在编排层 `record(target_kind="conversation"/"task")`。价值是把会议室编排也纳入统一体检与下钻。

### 4.5 其余（grader / security / doc_writer / spec_writer / summarizer）

按同一脚手架滚动跟进。security 有扫描器结果 → 双层；doc_writer/spec_writer 1→1 文本产物 → 记 `target_kind="cr"/"conversation"`。

---

## 5. 优化循环工具化（§3 表已就位，工具未建——本次深化重点）

结构化 + 版本化 + trace 链让下列闭环成立。**当前只有「看单条产出」，缺「跨产出聚合」**。补三件：

### 5.1 字段级体检（field health）— 新命令 + 面板

**后端**（`commands/agent_outputs.rs` 新增，薄包装 + 纯 async fn）：

```rust
// agent_output_field_health(role, schema_version?, project_id?) -> FieldHealth
struct FieldHealth {
  role, schema_version, total: u64,
  status_dist: { ok, partial, error },          // 解析健康度
  fields: Vec<{ path: String,                   // 形如 coverage.gaps
                fill_rate: f64,                  // 非空/非默认占比
                empty_rate: f64 }>,
  avg_confidence: Option<f64>,                   // 若 schema 有该字段
}
```

实现：拉该 (role, version) 的 `output_json` 行，在 **Rust 内遍历 JSON** 统计每个叶子字段的填充率
（空串/空数组/null/默认值算未填）。不依赖 SQLite JSON 扩展，跨平台稳。`total` 大时加 `LIMIT` 采样。

**用途**：某字段长期 `fill_rate` 低 → 暴露 prompt 没要到 / schema 设计过度 / 模型能力不足，是「改 schema 还是改 prompt」的客观依据。

### 5.2 版本 A/B 对比

同一 role 两个 `schema_version` 并存时，并排跑 5.1 的指标 + `status_dist` + 业务结论分布
（如 test 的 `verdict` 分布、triage 的 `needs_clarification` 率），决定是否切版本。
后端复用 5.1（传不同 version 各算一次），前端并列两栏。

### 5.3 失败回灌（replay）

查 `status IN ('error','partial')` 的行（已有 `list_agent_outputs` filter 支持 `status`）
→ 导出 `{prompt 上下文（raw 反推或 trace 取）, 期望 schema}` 作为 few-shot/修复样本，
喂回 prompt/schema 迭代，而不是凭感觉改提示词。与 `dirty-input-dogfooding.md` 的崩点清单对接：
脏输入崩点 → 落到具体 role 的具体字段 → 改 schema → 下一轮验证。

### 5.4 前端落点

复用 `Trace.tsx`「环节产出」tab：在其顶部加一个 seg「**产出浏览** / **schema 体检**」。
体检视图：role 下拉（`list_agent_output_roles`）→ 字段填充率条形 + status 饼 + 版本 A/B 并排。
样式只用 `src/index.css` 变量、图标走 `<Icon/>`、下拉用 `proj-select`（禁原生 select）。

---

## 6. 落地顺序（已完成 ✅ / 待办 ⬜）与验收

| # | 项 | 状态 | 验收 |
|---|----|------|------|
| 1 | 脚手架 `agents/schema.rs` + 迁移 `0040_agent_outputs` | ✅ | — |
| 2 | analysis dual-write + test 双层样板 | ✅ | `agent_outputs` 有 role=analysis/test 行 |
| 3 | 读取层 `commands/agent_outputs.rs` + 前端环节产出浏览器 | ✅ | Trace 页可筛选/下钻 |
| 4 | **批量扩展** `parse_array_or_empty`（§2.1）+ 规范样例/解析单测（§3.5） | ✅ | 数组解析 ok/partial/error 单测过；triage/proposer 样例单测过 |
| 5 | **triage 升级**（§4.1）：`TriageParsed` spec + 落库 + `needs_clarification` | ✅ | `run_system_role_text_traced` 带 trace 落 `agent_outputs(role=triage)`；denoise/idx 回退保留 |
| 6 | **proposer 升级**（§4.2）：`ProposalItem` + 结构化 `Evidence` + 一次运行一行落库 | ✅ | `agent_outputs(role=proposer,target=project)` 带结构化 evidence |
| 7a | **字段级体检**（§5.1）+ schema 体检面板（§5.4） | ✅ | Trace「schema 体检」tab：选 role 看字段填充率 + status 分布 |
| 7b | 版本 A/B（§5.2）+ 失败回灌（§5.3） | ⬜ | 两版本并排指标；导出 error/partial 样本 |
| 8 | code/planner/grader/security… 滚动接入（§4.3/4.4/4.5） | ⬜ | 各自 role 行入表 |

**进度**：4·5·6·7a 已于 2026-06-20 落地（cargo test 104 passed，npm build 通过）。剩 7b 与 8。

> 实现取舍：`record_each` 未单列，1→N 落库以 triage 的 `record_triage` 辅助 + 调用点循环 `record` 实现（更直观）；
> proposer 因 propose 时 issue 尚未创建，按「一次运行一行、target=project」沉淀（见 §4.2），per-issue 链接留作 §8 演进。
> trace 关联通过新增的 `llm::run_system_role_text_traced`（在内层 `scope_run` 复用同一 trace_id 后读出）实现。

---

## 7. 不变量与铁律对齐

- **纯 Rust、零 Tauri**：`agents/schema.rs`、各 spec、`intake/triage.rs`、`intake/proposer.rs`、`agent_outputs` 写入均不碰 `AppHandle`/`State`/事件；新命令保持薄包装（取 state → 调纯 async fn → 返回）。
- **落库 best-effort、绝不阻断主流程**：`record`/`record_each` 失败只记日志（已有约定）。闸口/流水线决策不得依赖 `agent_outputs` 写入成功。
- **迁移只增不改**：`0040` 已定；本轮无需新迁移（批量扩展与工具化纯代码）。
- **外部输入过滤不变**：proposer/triage 产出仍视为外部输入，回灌上下文前过 `has_obvious_injection()`。
- **schema 演进规则**：字段只增（带 `#[serde(default)]`）；破坏性改动必须 bump `VERSION` 并保持 reader 容错；历史行按其自带 `schema_version` 解释，旧数据始终可读（支撑版本 A/B）。
