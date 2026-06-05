# API 契约

## IPC 调用规范

前端统一使用 import { invoke } from '@tauri-apps/api/core' 调用后端，禁止使用 Tauri 1.x 的 @tauri-apps/api/tauri 路径。

---

## 事件推送规范

后端主动推送使用 Tauri Event，频道固定为 autoforge://event，payload 遵循 AppEvent 枚举格式，前端通过 listen 订阅。

---

## 输入安全过滤

所有来自外部的需求文本（Webhook、GitHub、用户输入）在入库前必须经过 core/security::has_obvious_injection() 过滤，不可绕过。

---

## API Key 存储

LLM API Key 当前存储在 SQLite llm_configs 表，后续迁移至 Tauri keychain plugin，禁止以明文写入前端代码或 tauri.conf.json。
