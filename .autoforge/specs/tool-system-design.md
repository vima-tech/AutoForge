# Tool 系统设计规范

> 最后更新：2025-01（基于 `src-tauri/src/agents/tools/` 实际实现）

## 1. 架构总览

Agent 可调用的工具来自**两个来源**，统一汇入 `ToolRegistry`，走同一条调用 + 安全过滤路径：

```
┌─────────────┐     ┌─────────────┐
│ 内置工具     │     │ MCP 外部工具 │
│ (BuiltinTool)│     │ (mcp.rs)    │
└──────┬──────┘     └──────┬──────┘
       │                   │
       ▼                   ▼
   build_registry_for_agent()   ← 合并入口
       │
       ▼
   ToolRegistry  →  llm.rs（按 provider 渲染 wire 格式）
       │
       ▼
   invoke()  →  安全闸（has_obvious_injection + 截断）→ 回灌上下文
```

## 2. 核心抽象

### 2.1 `ToolSpec` — 工具声明

```rust
pub struct ToolSpec {
    pub name: String,         // LLM 看到的 function name
    pub description: String,  // 工具描述
    pub parameters: Value,    // JSON Schema（type=object），空对象 = 无参数
}
```

由 `llm.rs` 在调用前按 `api_spec`（openai / anthropic）渲染成各自的 wire 格式。

### 2.2 `Tool` trait — 工具实例

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    async fn call(&self, args: Value) -> Result<String>;
}
```

实现者只产出文本结果；安全过滤由 `ToolRegistry::invoke` 统一施加。

### 2.3 `ToolContext` — 装配上下文

```rust
pub struct ToolContext {
    pub project_id: Option<String>,
    pub repo_root: Option<PathBuf>,
}
```

通过 `ToolContext::resolve(db, project_id)` 构造。不依赖项目的工具（如 `web_search`）忽略此上下文；代码扫描类工具在 `repo_root` 为空时不装配。

### 2.4 `ToolRegistry` — 注册表

| 方法 | 说明 |
|------|------|
| `new()` | 空注册表 |
| `register(tool)` | 注册一个 `Arc<dyn Tool>` |
| `specs()` | 返回所有工具的 `ToolSpec`（传给 LLM） |
| `invoke(name, args)` | 按名称调用，返回 `ToolOutcome`（含安全闸） |

## 3. 内置工具（Builtin Tools）

### 3.1 目录结构

```
src-tauri/src/agents/tools/
├── mod.rs          # 核心抽象 + 注册表 + 装配逻辑
├── web_search.rs   # WebSearchFactory — 联网搜索
├── code_scan.rs    # CodeScanFactory::{Read, Search, List} — 代码扫描
└── mcp.rs          # MCP 外部工具适配
```

### 3.2 当前内置工具清单

| 工具名 | 工厂 | 需要项目 | 说明 |
|--------|------|----------|------|
| `web_search` | `WebSearchFactory` | 否 | 联网搜索（Tavily / SearXNG） |
| `read_project_file` | `CodeScanFactory::Read` | 是 | 读取项目文件（2MB 上限） |
| `search_project_code` | `CodeScanFactory::Search` | 是 | 全文搜索项目代码 |
| `list_project_files` | `CodeScanFactory::List` | 是 | 列出项目文件清单 |

### 3.3 `BuiltinTool` trait — 工厂接口

```rust
#[async_trait]
pub trait BuiltinTool: Send + Sync {
    fn info(&self) -> ToolInfo;
    async fn build(&self, db: &Db, ctx: &ToolContext) -> Option<Arc<dyn Tool>>;
}
```

- `info()` → 返回 `ToolInfo { name, label, needs_project }`，驱动白名单匹配 + 前端开关渲染
- `build()` → 按上下文/配置装配工具实例；前提不满足时返回 `None` 跳过

### 3.4 注册入口（唯一登记处）

```rust
// src-tauri/src/agents/tools/mod.rs
pub fn builtin_catalog() -> Vec<Box<dyn BuiltinTool>> {
    vec![
        Box::new(web_search::WebSearchFactory),
        Box::new(code_scan::CodeScanFactory::Read),
        Box::new(code_scan::CodeScanFactory::Search),
        Box::new(code_scan::CodeScanFactory::List),
    ]
}
```

**新增内置工具只需两步**：
1. 在 `tools/` 下新建文件，实现 `BuiltinTool` trait
2. 在 `builtin_catalog()` 的 `vec![]` 里追加一行

注册、白名单门控、前端开关自动适配，无需改动装配逻辑。

### 3.5 前端元信息

`builtin_catalog_meta()` 返回 `Vec<ToolInfo>`，通过 IPC `list_builtin_tools` 暴露给前端 Settings 页面，动态渲染 Agent 能力开关。新增工具自动出现，无需改前端。

## 4. MCP 外部工具

### 4.1 数据来源

从 `mcp_servers` 表动态加载，字段：

| 字段 | 说明 |
|------|------|
| `name` | Server 名称 |
| `enabled` | 是否启用（仅 `enabled=1` 参与装配） |
| `transport` | `"stdio"` 或 `"http"` |
| `command` / `args_json` | stdio 模式的子进程命令和参数 |
| `url` / `headers_json` | http 模式的远程地址和自定义头 |
| `env_json` | stdio 模式的环境变量（加密存储） |
| `agent_ids_json` | 适用 Agent ID 列表（决定哪些 Agent 可用该 server 的工具） |

### 4.2 传输方式

| 传输 | 实现 | 说明 |
|------|------|------|
| `stdio` | `TokioChildProcess` | 启动本地子进程作为 MCP server |
| `http` | `StreamableHttpClientTransport` | 连接远程 MCP server（支持自定义 headers） |

### 4.3 命名规则

MCP 工具对外暴露名为 `mcp__<server_slug>__<tool_name>`，其中 slug 经 `sanitize()` 清洗为 `[A-Za-z0-9_-]` 字符集，避免与 LLM function name 规范冲突。实际调用时用 server 上的原始工具名（`remote_name`）。

### 4.4 生命周期

**Connect-per-turn**：每次为 Agent 构建注册表时按需连接其适用的 server，列出工具后连接句柄由 `McpTool` 以 `Arc` 持有，随注册表 drop 而关闭。不维护长连接池。

### 4.5 安全约束

- MCP 工具结果视为**不可信外部输入**，回灌上下文前由 `ToolRegistry::invoke` 统一过 `has_obvious_injection()` + 截断
- MVP 阶段只允许**只读/无副作用**工具，写类工具默认禁用并走白名单
- MCP 代码纯 Rust 零 Tauri 引用，符合后端独立化方向

## 5. 装配流程

`build_registry_for_agent(db, agent, ctx)` 的完整流程：

```
1. 创建空 ToolRegistry
2. 解析 Agent 的 capabilities_json → 得到 allowed_tools 白名单
3. 遍历 builtin_catalog()：
   ├─ 工具名不在白名单 → 跳过
   ├─ factory.build(db, ctx) 返回 None → 跳过（配置/上下文不满足）
   └─ 注册进 registry
4. 查询 mcp_servers WHERE enabled=1：
   ├─ agent_id 不在 server.agent_ids_json → 跳过
   ├─ connect_tools(server) 失败 → warn 日志，跳过
   └─ 逐个注册进 registry
5. 返回 ToolRegistry
```

## 6. 新增工具 Checklist

### 内置工具

- [ ] 在 `src-tauri/src/agents/tools/` 新建文件（或扩展现有文件）
- [ ] 实现 `BuiltinTool` trait（`info()` + `build()`）
- [ ] 实现 `Tool` trait（`spec()` + `call()`）
- [ ] 在 `builtin_catalog()` 追加一行
- [ ] 验证：前端 Settings 页面自动出现新开关
- [ ] 验证：Agent 勾选后工具可正常调用

### MCP 工具

- [ ] 在 Settings 页面添加 MCP Server 配置（transport / command / url 等）
- [ ] 勾选适用 Agent
- [ ] 启用 server
- [ ] 验证：Agent 调用时自动发现并注册该 server 的工具