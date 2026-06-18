# Schema 驱动的 Agent(执行标准 + 优化记录 一体化)

> 目标:把每个环节 agent 从「自由 prompt → 自由文本」升级为「**版本化 schema** 既约束推理、又结构化沉淀产出」。
> schema 一物三用:**① 执行标准**(强制 agent 覆盖全部分析角度 → 更稳更强)、**② 优化信息源**(结构化记录,优化时拿全面信息而非一坨文本)、**③ 优化杠杆**(持续打磨 schema 即持续优化 agent)。

---

## 0. 现状盘点

| 环节 / 角色 | 现在的产出 | schema 状态 |
|------------|-----------|------------|
| **analysis(需求分析)** | `IssueAnalysisSpec` v1.0 强类型 + `issue_analyses` 落库 + 独立 `.schema.json` | ✅ **完备样板** |
| planner(调度器) | `ConversationPlan` JSON | 🟡 半结构化 |
| **test(测试)** | `tasks/testing.rs` 跑配置 shell 命令,写 `test_sessions.results_json`;`PROMPT_TEST` LLM 输出格式甩给"调用方" | 🔴 无 schema,推理未结构化 |
| triage / proposer / code / grader / security / doc_writer / spec_writer / summarizer / … | 自由文本 `PROMPT_*` | 🔴 无 |

结论:**analysis 已经把范式跑通了**,缺的是(a)把它的脚手架抽成可复用件,(b)按价值顺序推广到其余环节,(c)把结构化产出统一沉淀以支撑"优化循环"。

---

## 1. 范式拆解(从 `analysis.rs` 提炼)

一个 schema 驱动 agent = 5 件套:

1. **schema JSON**:`agents/schemas/<role>.schema.json`,带 `schema_version`。
2. **强类型 Rust spec**:serde structs,**所有字段 `#[serde(default)]`**(前向兼容:模型漏字段不崩,只降质量)。
3. **prompt 内嵌 schema 作执行标准**:`SYSTEM_PROMPT` 把 schema 摊开 + "严格只输出 JSON 对象"。
4. **健壮解析**:抽最外层 `{...}` → 类型化反序列化 → 旧格式 fallback → 数值 clamp → 失败用 default(不阻断主流程)。
5. **结构化落库 + 版本戳**:热点字段建列 + 全量 `*_json` blob + `schema_version`。

---

## 2. 要建的可复用脚手架(让每个新 agent 变便宜)

把 `analysis.rs` 里的样板下沉到新模块 **`agents/schema.rs`**(纯 Rust,零 Tauri):

- `extract_json(text) -> Option<&str>`(从 analysis.rs 搬出共用)。
- `parse_or_default<T: DeserializeOwned + Default>(text) -> (T, status)`:统一"最外层括号 + 类型化 + 失败 default"。
- `trait StructuredOutput`:`schema_version()` / `schema_json()` / `prompt_contract() -> String`(把 schema 渲染成 prompt 块,**执行标准与落库结构同源**,杜绝 prompt 与 struct 漂移)。
- `record(db, AgentOutput)`:统一写入 `agent_outputs`(见 §3)。

> 收益:新增一个 schema agent ≈ 写一份 schema + 一组 struct + 一段 prompt,解析/落库/trace 复用脚手架。

---

## 3. 统一结构化产出表 `agent_outputs`(= 记账本/trace 模板的正确落点)

> 这张表把前面讨论的「记账本」一般化:**每个环节 agent 的结构化产出都按统一信封沉淀,可按需求/CR 串成流水线全貌,并链回 `llm_traces` 做单步下钻。** 这就是"方便定位问题 + 优化时拿全面信息"的载体。

新增**一张**迁移(不可逆,一次定对):

```sql
CREATE TABLE agent_outputs (
  id            TEXT PRIMARY KEY,
  role          TEXT NOT NULL,         -- analysis | test | triage | proposer | ...
  schema_version TEXT NOT NULL,
  target_kind   TEXT NOT NULL,         -- issue | cr | task | conversation
  target_id     TEXT NOT NULL,
  project_id    TEXT,
  trace_id      TEXT,                  -- 链回 llm_traces(单步推理下钻)
  status        TEXT NOT NULL,         -- ok | partial | error
  output_json   TEXT,                  -- 该 schema 的完整结构化产出
  raw           TEXT,                  -- 原始模型文本(审计)
  created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);
-- 索引:target、role+version、trace_id、created_at
```

设计要点:
- **additive,不动现有表**。analysis 继续写 `issue_analyses`(向后兼容),同时可 dual-write 到 `agent_outputs` 做统一视图;其余新 agent 直接只用 `agent_outputs`。
- `trace_id` 是与现有 `llm_traces` 的桥:记账本看"哪个环节产出有问题" → 点 `trace_id` 进 Trace 详情看"那一步模型怎么想的"。两层粒度各司其职(流水线级 vs 单调用级)。

---

## 4. 推广顺序(按战略价值排,不是按难度)

紧扣"脏输入→外包上游"主线:**入口炼化 + 流水线闸口**最该先上。

1. **test agent(闸口,现状最弱)** —— schema 两段:
   - *设计段*:测试用例(前置/输入/操作/预期)、覆盖映射到 analysis 的 `acceptance_criteria`、高风险回归点、覆盖缺口。
   - *诊断段*:失败根因、最小复现、期望 vs 实际、修复方向、`verdict(pass|fail|flaky)`。
   - 让 `tasks/testing.rs` 的命令结果 + LLM 诊断都进同一 schema,gate 决策有结构依据。
2. **triage agent(炼噪声为需求)** —— 正是脏输入实验压的入口。schema:精炼后需求、`clarity_score`、`missing_info[]`、`needs_clarification(bool)`(对应"静默猜 vs 追问"红线)、分类、去重。**直接服务外包场景的非结构化入料。**
3. **proposer(自动供料)** —— schema:改进提议 + `file:line` 证据 + 影响面 + 工作量,喂工厂自走。
4. **code/execution 结果** —— schema:改了什么+为什么、DoD 自检清单、自审风险,让审核2更快。
5. 其余(grader / security / doc_writer / spec_writer …)按需跟上同一脚手架。

---

## 5. 优化循环(为什么"持续优化 schema = 优化 agent")

结构化 + 版本化 + trace 链使下列闭环成立:

- **字段级体检**:统计某 role 哪些字段长期空着/低质 → 暴露 schema 或 prompt 的弱点。
- **版本 A/B**:`schema_version` 并存,对比新旧版产出质量再切换。
- **回灌**:把结构化失败样本喂回 prompt/schema 迭代,而不是凭感觉改提示词。
- **与 dogfooding 联动**:脏输入实验的崩点 → 落到具体 role 的具体字段 → 改 schema → 下一轮验证。(见 `dirty-input-dogfooding.md`)

---

## 6. 落地顺序建议

1. 建脚手架 `agents/schema.rs` + 迁移 `00NN_agent_outputs.sql`(一次定对表结构)。
2. 实现 **test agent** schema 作第 2 个样板(验证脚手架够通用)。
3. 接 **triage**(战略最高:外包入料)。
4. 前端:Trace 页加「环节产出」视图,按 `target_id` 串流水线 + 点 `trace_id` 下钻。
5. 其余 agent 滚动跟进;每个 schema 在 `agents/schemas/` 留独立 `.schema.json` 并登记版本。

> 铁律对齐:`agents/schema.rs`、各 spec、`agent_outputs` 写入均为纯 Rust,不碰 Tauri 类型;命令保持薄包装;迁移只增不改。
