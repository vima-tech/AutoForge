import { getCurrentWindow } from '@tauri-apps/api/window';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

// 双击顶部栏（标题栏 / 阅读模式 reader-bar）切换窗口最大化/还原。
// 忽略落在按钮、链接、输入控件上的双击，避免与工具按钮冲突。
export function toggleMaximizeOnDoubleClick(e: React.MouseEvent<HTMLElement>) {
  if (!isTauri) return;
  if ((e.target as HTMLElement).closest('button, a, input, textarea, select')) return;
  void getCurrentWindow().toggleMaximize();
}
