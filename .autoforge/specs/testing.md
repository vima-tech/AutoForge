# 测试要求

## IPC 功能测试环境

涉及 Tauri IPC、窗口控制、文件系统的功能必须通过 npm run tauri:dev 在完整 Tauri 环境中测试，npm run dev 浏览器模式不可替代。

---

## 合并前自动测试

流水线 review_2 通过后，自动执行测试后再触发 git merge --no-ff 合并到 dev 分支，测试失败须阻断合并。

---

## 并发槽位验证

并发控制测试需验证：同时执行 CR 数不超过 Semaphore 上限（默认 5），pending_review 达到阈值（默认 20）时新任务暂停入队。
