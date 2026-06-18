# 自定义 LLM 适配器图片输入（Vision）支持

| 字段 | 值 |
|------|----|
| 状态 | 待实现（提案 v2，2026-06-18 复核仍成立） |
| 优先级 | P2（中高 — 真实能力缺口，硬报错挡住合法用法） |
| 涉及层 | 后端 `agents/llm.rs`（纯 Rust，零 Tauri） |
| 工作量 | 中（anthropic + openai 两种 wire 格式各加图片块编码 + 把 image_paths 透传进分发函数，约 0.5–1 天） |
| 相关 | `agents/llm.rs`、`conversation_attachments` 表、[[AutoForge-操作者身份卡与通知中心-功能提案-v2]] |

---

## 1. 背景与问题

会议室支持图片附件（`conversation_attachments`，白名单 MIME、≤10MB），
但**自定义 LLM 适配器目前完全不接受图片输入**——`agents/llm.rs:54-58` 直接硬报错：

```rust
if !image_paths.is_empty() {
    return Err(anyhow!(
        "当前 LLM 适配器暂不支持图片输入，请改用 Claude CLI 或移除图片附件"
    ));
}
```

后果：在「省钱走自定义 LLM」的策略下（见记忆 [[project_llm_routing_policy]]，
绝大多数角色 Agent 都绑自定义 LLM 而非 Claude CLI），**只要用户在群聊里发图，
任务就直接失败**，只能改用更贵的 Claude CLI 或删掉图片。这是个明显的能力短板。

而工具循环 `run_agent_text_with_tools_inner`（`llm.rs:116`）在「带图片输入」时还会退化为无工具单轮
（`registry.is_empty() || !image_paths.is_empty() || agent.llm_id.is_none()` → 走老路径），
进一步放大了图片场景下的能力损失。

## 2. 目标 / 非目标

**目标**
- 让 `anthropic` 与 `openai` 两种 wire 格式（`llm.rs:61-63` 分发的唯一两种）都能把图片附件
  作为多模态输入传给模型，移除一刀切的硬报错。
- 对确实不支持 vision 的模型/端点，保留优雅降级（跳过图片 + 提示），而非整体失败。

**非目标**
- 不改 Claude CLI 路径（其图片能力已正常）。
- 不在本提案处理音频/视频等其它模态（语音录入已由 `agents/asr.rs` 单独覆盖）。

## 3. 方案

`run_agent_text(...)` 已经拿到 `image_paths`（`llm.rs:31`），但当前 `run_anthropic` / `run_openai_compatible`
（`llm.rs:304` / `:251`）签名里**只有 (cfg, prompt, system_prompt)，没有把图片透传进去**。改造：

1. 把 `image_paths` 透传进两个分发函数（及其工具循环版 `run_anthropic_tool_loop` / `run_openai_tool_loop`）。
2. 按 wire 格式编码：
   - **anthropic**：图片读 base64，作为 `content` 数组里的
     `{"type":"image","source":{"type":"base64","media_type":..,"data":..}}` 块，与文本块并列。
   - **openai**：`content` 数组 + `{"type":"image_url","image_url":{"url":"data:<mime>;base64,<...>"}}` 块。
3. 读图前复用附件白名单/大小校验（与 `conversation_attachments` 入库一致），防止把超大/非白名单文件塞进请求。
4. 模型是否支持 vision 不易静态判定——采用「**尝试发送，端点报不支持则降级为纯文本单轮 + 在输出里标注"已忽略 N 张图片"**」，
   与现有工具循环「出错即回退」的容错风格保持一致。
5. 放开工具循环「带图片即退化」的限制，使 vision 与工具调用可正交（按模型能力决定）。

## 4. 验收标准

1. 绑定 anthropic 兼容 vision 模型的角色群聊里发图 + 文本，Agent 能基于图片内容作答，不报错。
2. openai 兼容 vision 模型同上。
3. 绑定不支持 vision 的端点时，任务不再整体失败，而是忽略图片继续作答并明确提示。
4. 纯文本任务行为不回归。

## 5. 风险与缓解

- **请求体积膨胀 / 超 token**：限制单次随上下文携带的图片数量与单图大小（沿用 10MB 上限，必要时再压缩）。
- **不可信外部输入**：图片本身不过 `has_obvious_injection`（那是文本注入检测），
  但模型对图中文字的解读结果若回灌上下文，仍按现有「外部输入」策略处理。
- **端点能力差异**：以容错降级兜底，不做脆弱的「模型名白名单」硬判断。
