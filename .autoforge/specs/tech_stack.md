# 技术栈

## Tauri 版本锁定

必须使用 Tauri 2.11.2（Rust crate）与 @tauri-apps/api 2.11.0，禁止参考或混用 Tauri 1.x 的任何 API、文档或示例。

---

## 前端技术约束

前端使用 React 18.3 + TypeScript 5.6 + Vite 5.4，不引入其他 UI 框架（如 MUI/Antd）。

---

## 后端运行时

后端全量使用 Rust（async/tokio），不引入 Node.js 服务或 Python 脚本作为后端逻辑载体。

---

## 数据库选型

持久化层统一使用 SQLite（sqlx），禁止引入 Redis、PostgreSQL 等外部数据库依赖，保持零外部依赖。

---

## AI 执行环境

代码实现任务通过本地 claude CLI 在独立 git worktree 中执行，不直接调用 OpenAI Codex 等云端代码生成 API。
