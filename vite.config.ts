import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;
// 单一端口真源：测试环境可通过 PORT / AUTOFORGE_DEV_PORT 指派端口，本地默认 1420。
// tauri.conf.json 的 devUrl 由 tauri:dev 脚本注入同一端口，二者必须一致。
const devPort = Number(process.env.PORT ?? process.env.AUTOFORGE_DEV_PORT) || 1420;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: devPort,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: devPort + 1 }
      : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: process.env.TAURI_ENV_PLATFORM === "windows"
      ? "chrome105"
      : "safari13",
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
