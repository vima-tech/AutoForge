// 跨平台 `tauri dev` 启动器。
//
// 为什么需要它：原 `tauri:dev` npm 脚本用 bash 的 `${PORT:-${AUTOFORGE_DEV_PORT:-1420}}`
// 语法把端口注入 Tauri 的 devUrl，而 Windows 上 npm 经 cmd.exe 执行脚本，不支持
// `${VAR:-default}` 展开 —— 会把字面量当端口传入，导致 devUrl 失效。改用 Node
// 计算端口（与 vite.config.ts 同一真源），跨平台一致。
import { spawn } from "node:child_process";
import { writeFileSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// 单一端口真源：PORT / AUTOFORGE_DEV_PORT，默认 1420（与 vite.config.ts 完全一致）。
const port = Number(process.env.PORT ?? process.env.AUTOFORGE_DEV_PORT) || 1420;

// 配置走临时文件而非内联 JSON：避免 JSON 内的双引号在 cmd/sh 下被破坏。
// `--config` 同时接受「JSON 字符串」或「JSON 文件路径」，这里用路径最稳。
const cfgPath = join(tmpdir(), `autoforge-tauri-dev-${process.pid}.json`);
writeFileSync(cfgPath, JSON.stringify({ build: { devUrl: `http://localhost:${port}` } }));

// shell:true 让 Windows 经 cmd 解析 `tauri.cmd` shim（node_modules/.bin 已在
// npm 注入的 PATH 中）；路径加双引号以兼容含空格的临时目录，两端 shell 通用。
const child = spawn(`tauri dev --config "${cfgPath}"`, { stdio: "inherit", shell: true });

const cleanup = () => {
  try {
    unlinkSync(cfgPath);
  } catch {
    /* 已删除或不存在，忽略 */
  }
};
child.on("exit", (code) => {
  cleanup();
  process.exit(code ?? 0);
});
process.on("SIGINT", () => child.kill("SIGINT"));
process.on("SIGTERM", () => child.kill("SIGTERM"));
