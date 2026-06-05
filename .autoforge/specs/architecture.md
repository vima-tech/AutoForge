# 架构约束

## 双审核节点

流水线必须包含两个人工审核节点：审核 1（需求分析后）和审核 2（代码实现后），任何自动合并必须经过 review_2 approved 分支触发。

---

## IPC 单一入口

前端所有后端调用必须通过 src/services/index.ts 封装，禁止在页面组件中直接调用 invoke，确保 IPC 可追踪、可 mock。

---

## 工作区写入限制

Agent 写文件仅允许操作项目 .autoforge/docs/ 和 .autoforge/specs/ 目录，workspace.rs 强制校验路径，禁止 .. 路径越界。

---

## 任务队列架构

后台任务通过进程内 Tokio mpsc channel 调度，不使用 Redis 或外部队列，任务幂等键写入 job_executions 表防止重复执行。

---

## Git 安全代理

所有 git 操作必须经 GitProxy 拦截，禁止 push main/master、push --force、config --global 等危险命令直接执行。
