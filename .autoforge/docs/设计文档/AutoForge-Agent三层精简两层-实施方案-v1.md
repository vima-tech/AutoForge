# AutoForge Agent 三层精简为两层 · 实施执行方案 v1

> 目标：把「LLM 配置 / Agent 配置 / 角色指派」**三页三层**精简为
> **「LLM 配置 / 角色」两层**；系统角色自带内置提示词（可补充），自定义对话角色
> 完整保留用于群聊。本方案供审核，确认后再执行。

---

## 1. 目标与原则

- **两层**：第 1 层 `LLM 配置`（不变）；第 2 层 `角色`＝合并掉「Agent 配置 + 角色指派」。
- 第 2 层两支：
  - **系统角色**（内置目录、固定槽位）：只需「选 LLM + 提示词(内置/补充) + 启用」。
  - **对话角色**（用户自建、群聊用）：无限创建，名字/头像/自定义提示词/LLM/可@/capabilities 全保留。
- **提示词内置 + 可补充**：内置基线收口到代码注册表（单一真源），用户默认零配置；可"内置+补充"或"完全自定义"。
- **不破坏现状**：表结构增量扩展、命令向后兼容、分阶段上线，每阶段可独立验收回滚。

---

## 2. 现状基线（改造前）

- 设置页 `SET_ITEMS`（`src/pages/Settings.tsx`）含独立三项：`llm` / `agents` / `roles`。
- `agents` 表字段：`id,name,name_en,role,color,initial,llm_id,system_prompt,forge_role,role_type,system_kind,capabilities_json,max_concurrency,visible_in_chat,mentionable,enabled,created_at`。
- 角色解析：`run_system_role_text(db,kind,…)` 按 `system_kind` LIKE 取 enabled 首个；**有自定义 system_prompt 则整段替换，否则用调用方传入的兜底**。
- 内置提示词现状散落在：seed 迁移（0011/0012/0017）的 `system_prompt` 列 + 各调用处的 fallback 字符串（如 `prototype.rs`、`grader.rs`）。
- 命令：`list_agents/create_agent/update_agent/delete_agent/set_agent_forge_role`、`list_llm_configs/...`。
- 代码引用的全部职责：system_kind ×10（planner/summarizer/doc_writer/context_compressor/material_ai/spec_writer/grader/security/deploy/prototype）+ forge_role ×2（analysis/test）。

---

## 3. 目标模型

```
第 1 层 LLM 配置        provider/model/key/endpoint          ← 不变
第 2 层 角色 Role-Agent
   ├─ 系统角色（内置目录，来自代码注册表 ROLE_REGISTRY）
   │    卡片：启用 · 选 LLM · 提示词模式[内置/内置+补充/自定义] · (补充/自定义文本)
   │    覆盖 12 个职责：10 个 system_kind + analysis/test(forge_role)
   └─ 对话角色（自定义业务 Agent，群聊用）
        [＋新建] 名字/头像/颜色 · 选 LLM · 提示词(模板或自由) · 可@ · capabilities
```

提示词最终拼接：
```
final_system_prompt =
  builtin (来自 ROLE_REGISTRY[kind])           当 mode='builtin'
  builtin + "\n\n# 补充约束\n" + supplement     当 mode='append'
  custom_text                                    当 mode='custom' 或 无注册表项(纯自定义对话角色)
```

---

## 4. 数据模型变更（迁移，仅新增列）

新增迁移 `migrations/0025_agent_prompt_mode.sql`（不可改旧迁移）：

```sql
ALTER TABLE agents ADD COLUMN prompt_mode TEXT NOT NULL DEFAULT 'custom';
-- 'builtin' | 'append' | 'custom'
-- 兼容策略：已有 Agent 一律置 'custom'，保持其现有 system_prompt 行为不变。
-- 全新 seed 的系统角色用 'builtin'。
```

> 说明：`system_prompt` 列语义扩展为"补充或自定义文本"，由 `prompt_mode` 决定怎么用；不删字段、不改旧数据值，零行为漂移。

---

## 5. 后端改造

### 5.1 新增内置角色注册表 `src-tauri/src/agents/roles.rs`

单一真源，描述每个内置角色：

```rust
pub struct RoleDef {
    pub kind: &'static str,         // system_kind 或 'analysis'/'test'
    pub name: &'static str,         // 设计原型师
    pub name_en: &'static str,      // Design Prototyper
    pub group: RoleGroup,           // Orchestration | Delivery | Pipeline
    pub binding: RoleBinding,       // SystemKind | ForgeRole
    pub builtin_prompt: &'static str,
    pub default_caps: &'static str, // capabilities_json
    pub color: &'static str,
    pub icon: &'static str,
    pub desc: &'static str,
}
pub static ROLE_REGISTRY: &[RoleDef] = &[ /* 12 条 */ ];
pub fn builtin_prompt(kind: &str) -> Option<&'static str>;
pub fn find(kind: &str) -> Option<&'static RoleDef>;
```

把现有散落的内置提示词（seed SQL + 各 fallback 串）**集中搬到这里**。

### 5.2 提示词拼接：`agents/llm.rs`

- `run_system_role_text(db, kind, prompt, fallback)`：解析到 agent 后，按 `agent.prompt_mode` 组合：
  - `builtin` → `roles::builtin_prompt(kind)`（注册表缺失时退回 `fallback`）。
  - `append` → builtin + agent.system_prompt。
  - `custom` → agent.system_prompt（空则退回 builtin/fallback）。
- `run_agent_text`（业务/对话角色，orchestration 调用处）：对话角色无注册表项 → 用其 `system_prompt`（custom）。保持现状即可，仅在有 kind 时复用同一拼接 helper。
- 抽一个 `compose_system_prompt(agent, builtin: Option<&str>) -> String` 公共函数，两处共用。

### 5.3 命令层 `commands/settings.rs`（新增，旧的保留）

- `list_role_catalog() -> Vec<RoleSlot>`：把 `ROLE_REGISTRY` 与当前 `agents` 表 join，返回每个系统角色的：kind/name/desc/group + 当前持有 agent（id/llm_id/prompt_mode/system_prompt/enabled）或"未指派"。驱动系统角色卡。
- `set_role_slot(kind, payload{ llm_id, prompt_mode, supplement, enabled })`：
  - 若该 kind 尚无持有 agent → 依注册表 `INSERT` 一个系统 Agent（name/name_en/icon/color/caps 取注册表默认，system_kind=kind）。
  - 若已有 → `UPDATE` 其 llm_id/prompt_mode/system_prompt/enabled。
  - 保证**单一持有**（沿用现 `setSystemRoleAgent` 的"摘掉其他持有者"逻辑）。
  - forge_role（analysis/test）走同一命令，内部改写 `forge_role` 而非 `system_kind`。
- **保留** `create_agent/update_agent/delete_agent`：服务于"对话角色"（自定义业务 Agent）。新增可选入参 `prompt_mode`。
- `set_agent_forge_role` 标记 deprecated（被 `set_role_slot` 取代），但保留以兼容。
- 注册到 `lib.rs invoke_handler`。

### 5.4 services 封装 `src/services/index.ts`

新增 `listRoleCatalog()`、`setRoleSlot(kind,payload)`；`createAgent/updateAgent` 增加 `prompt_mode` 可选字段。

---

## 6. 前端改造（三页 → 两页）

### 6.1 新「角色」页（替换 `roles`，并吸收 `agents`）

`src/pages/Settings.tsx` 新组件 `RolesPage`，两段：

- **系统角色**（数据来自 `listRoleCatalog`，按 group 分「群聊编排 / 交付与项目 / 流水线」三组）：每行一张 `RoleCard`：
  - 启用开关 · LLM `Select` · 提示词模式 `seg`[内置 / 内置+补充 / 自定义] · 展开补充/自定义 `textarea`（仅后两种显示）。
  - onChange → `setRoleSlot(kind, …)`。
- **对话角色**（数据 `listAgents` 里 `role_type==='business'`）：卡片列表 + `[＋新建对话角色]`：
  - 复用现 `AgentSettings` 的编辑表单（名字/头像/颜色/LLM/可@），提示词改为"模板下拉 + 自由编辑"（mode 默认 custom）。
  - 起手模板常量（通用助手/评审/文案/分析）。

### 6.2 `SET_ITEMS` 调整

- 删除 `agents`、`roles` 两项，新增单项 `roles`→「角色」。保留 `llm`。
- 顺序：`theme, llm, roles, concurrency, security, webhook, notify, gating, specs, about`。

### 6.3 复用与样式

- 复用现有 `panel/assign-row/cfg-logo/seg/field/proj-select(Select)` 类，**不新增平行样式**（遵守 DESIGN.md）。
- 图标走 `<Icon>`；颜色用 CSS 变量。

---

## 7. 数据迁移与兼容

1. `0025` 迁移加列，已有 Agent `prompt_mode='custom'` → 行为零变化。
2. 现有系统 Agent（planner 等）：保持其 system_kind 与 system_prompt 不动，继续 custom 模式；用户可在新卡片里一键切到 `builtin` 采用注册表基线。
3. 现有业务 Agent（analyst/coder/tester/architect/security）→ 自动出现在「对话角色」段（`role_type=business`）。
4. forge_role analysis/test → 在系统角色「流水线」组以卡片呈现，后端解析逻辑不变。
5. 不删除 `set_agent_forge_role`/旧命令；旧前端引用在切换后移除。

---

## 8. 分阶段执行（每阶段可独立验收 / 回滚）

| 阶段 | 内容 | 产出 | 风险 |
|------|------|------|------|
| **P0** | 建 `roles.rs` 注册表，把内置提示词收口；`compose_system_prompt` helper；`run_system_role_text` 改用注册表做兜底（**行为等价**） | 后端重构，无 UI 变化 | 低 |
| **P1** | 迁移 `0025` 加 `prompt_mode`；拼接按模式生效；新增 `list_role_catalog`/`set_role_slot` + services | 命令就绪，老 UI 仍可用 | 中 |
| **P2** | 新「角色」页（系统角色卡 + 对话角色段）；`SET_ITEMS` 合并 | 两层 UI 上线 | 中 |
| **P3** | 移除旧 `AgentSettings`/`RoleAssignment` 残留与无用引用；文档同步 | 清理 | 低 |

> 建议先合并执行 **P0+P1**（纯后端、可 `cargo check` 验证、不动界面），审核通过后再做 **P2** 界面。

---

## 9. 验收清单

- [ ] `cargo check` + `tsc && vite build` 通过。
- [ ] 全新库：系统角色未指派时，`list_role_catalog` 列出全部 12 个，选 LLM 即用（内置提示词）。
- [ ] 旧库升级：现有系统/业务 Agent 行为不变（prompt_mode=custom）。
- [ ] `prototype` 等四角色无需手动建 Agent 即可在卡片直接选 LLM 启用。
- [ ] 对话角色仍可新建、群聊 @、被 Planner 编排、写 `.autoforge`。
- [ ] 一个系统角色同一时刻仅一个持有者。
- [ ] 提示词三模式（内置/内置+补充/自定义）实际生效（可在 raw 输出/日志核对）。

---

## 10. 待你拍板的决策点

1. **旧系统 Agent 的提示词归属**：默认 `custom`（保留现有自定义，最安全）✔ 推荐；还是迁移时若与 seed 基线一致就改 `builtin`（更"干净"但有行为变动风险）。
2. **是否保留"一个 Agent 兼多角色"**：本方案默认**舍弃**（每系统角色独立卡，更清晰）。如需保留，系统卡的 LLM 选择改为"可选已有对话角色"，复杂度上升。
3. **对话角色能否兼任系统槽位**：v1 默认**否**（系统角色用各自专属 Agent）。如要"用我的自定义角色当 grader"，P2 再加一个"用自定义角色"逃生口。
4. **范围**：先做 P0+P1（后端），还是一次到 P2（含界面）。

---

## 11. 影响文件一览

| 层 | 文件 | 动作 |
|----|------|------|
| 迁移 | `migrations/0025_agent_prompt_mode.sql` | 新增 |
| 后端 | `agents/roles.rs` | 新增（注册表+内置提示词） |
| 后端 | `agents/mod.rs` | 导出 roles |
| 后端 | `agents/llm.rs` | `compose_system_prompt`，按模式拼接 |
| 后端 | `commands/settings.rs` | `list_role_catalog`/`set_role_slot`，create/update 加 prompt_mode |
| 后端 | `lib.rs` | 注册新命令 |
| 后端 | `models/agent.rs` | `Agent` 加 `prompt_mode` 字段 |
| 前端 | `services/index.ts` | `listRoleCatalog`/`setRoleSlot`，类型补 prompt_mode |
| 前端 | `pages/Settings.tsx` | 新 `RolesPage`（系统角色卡+对话角色段），改 `SET_ITEMS`，移除旧 `RoleAssignment`/`AgentSettings` |
| 文档 | 本文件 + `CLAUDE.md` Agent 章节 | 同步 |
