import { useEffect, useRef } from 'react';
import Icon from './Icon';

// 统一的「系统内提示框」：固定右下角浮层，替代浏览器原生 alert()。
// 视觉沿用既有 toast 风格（参考 Projects 页），仅引用 index.css 设计变量。
export type ToastTone = 'error' | 'success' | 'info';
export interface ToastData { msg: string; tone?: ToastTone }

const TONE: Record<ToastTone, { icon: string; color: string }> = {
  error: { icon: 'alert', color: 'var(--red)' },
  success: { icon: 'check', color: 'var(--green)' },
  info: { icon: 'brain', color: 'var(--ember)' },
};

export default function Toast({ data, onClose, duration = 4200 }: {
  data: ToastData | null; onClose: () => void; duration?: number;
}) {
  // onClose 走 ref，避免父组件无关 re-render 重置自动关闭计时
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  useEffect(() => {
    if (!data) return;
    const id = window.setTimeout(() => closeRef.current(), duration);
    return () => window.clearTimeout(id);
  }, [data, duration]);

  if (!data) return null;
  const tone = TONE[data.tone ?? 'info'];
  return (
    <div
      role="status"
      style={{
        position: 'fixed',
        right: 'calc(var(--win-gutter, 0px) + 28px)',
        bottom: 'calc(var(--win-gutter, 0px) + 28px)',
        zIndex: 260, maxWidth: 360,
        display: 'flex', alignItems: 'flex-start', gap: 10, padding: '11px 13px',
        borderRadius: 8, border: '1px solid var(--border-strong)', background: 'var(--bg-2)',
        boxShadow: 'var(--shadow-lg)', color: 'var(--text-2)',
        fontSize: 'var(--text-label)', lineHeight: 'var(--leading-normal)',
      }}
    >
      <Icon name={tone.icon} size={15} style={{ color: tone.color, flexShrink: 0, marginTop: 1 }} />
      <span style={{ minWidth: 0, wordBreak: 'break-word' }}>{data.msg}</span>
      <button className="icon-btn" style={{ width: 20, height: 20, flexShrink: 0 }} onClick={onClose} title="关闭提示" aria-label="关闭提示">
        <Icon name="x" size={11} />
      </button>
    </div>
  );
}
