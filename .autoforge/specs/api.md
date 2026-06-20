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

所有密钥（LLM api_key、MCP env/headers、web_search api_key）经 core/secrets.rs 信封加密落库：主密钥存系统钥匙环（keyring crate，无钥匙环时退化为 0600 本地文件），密钥本体以 AES-256-GCM 加密为 enc:v1: 密文存 SQLite。禁止以明文写入前端代码、tauri.conf.json 或直接落库（写入必经 secrets::encrypt_field，读取必经 secrets::decrypt）。
