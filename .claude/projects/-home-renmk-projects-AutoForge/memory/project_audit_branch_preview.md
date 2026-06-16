---
name: project_audit_branch_preview
description: 功能审计页改版——左侧按分支启动（worktree 隔离/多并行）、右侧 audit-right 移除、底部 dock
metadata:
  type: project
---

功能审计页（`src/pages/Audit.tsx` + `src-tauri/src/commands/cr_preview.rs`）2026-06 重构：

- **左侧「启动项目」改为选分支启动**：下拉列本地分支（`list_local_branches`，GitProxy `git branch --format`），main 标「线上」。支持**多分支并行**运行，左侧列出运行中的分支（`list_branch_previews`，按 `branch:<project>:<branch>` 键于 `state.dev_servers`）。命令：`start_branch_preview` / `stop_branch_preview` / `get_branch_preview_log`。
- **分支启动用隔离 worktree**（不在主仓库原地 checkout，避免自管理时崩溃）：`git worktree add --detach <path> <branch>`（detached 规避「branch already checked out」），路径 `worktrees_base()/branch/<project>/<branch>`，**复用**（stop 只杀进程不删 worktree）。**关键**：把主仓库 `node_modules` 软链进 worktree（gitignore 不随 worktree 走），否则 dev server 起不来。随机空闲端口 `free_port_from` + 导出 `PORT` 避冲突。web→dev server，tauri→app_command。
- **audit-right 整块移除**（连同 ResizeHandle#2、收起 rail、`renderRunEnv`、`run-env*`/`advice-wrap*`/`audit-right*` CSS）。预览与建议下沉到 **audit-left 底部悬浮 dock**（`.audit-dock`）：左 = 本次改动预览启动（`renderCrLaunch`，合并/no_session 时隐藏），右 = 管理员建议输入 + 修改按钮。
- 已删除上一版的 `launch_prod_app`/`get_prod_app_log`（生产 main 浏览器跳转/启动应用）——线上版本即用分支启动 main 代替。
- 合并后 worktree 目录已删：`load_ctx` 用 `.filter(path exists)` 把失效 session 视作 no_session，故「本次改动」预览自动隐藏。

相关：[[project_page_architecture]] [[project_self_managed_merge]]
