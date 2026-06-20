// 统一的时间解析/格式化工具。
//
// 后端时间戳有两种来源：
//   1. chrono::Utc::now().to_rfc3339()  → "2024-06-16T02:00:00+00:00"（带时区，正确）
//   2. SQLite datetime('now')           → "2024-06-16 02:00:00"（UTC，但无时区标记、空格分隔）
// 浏览器 new Date() 会把第 2 种当作「本地时间」解析，导致东八区少 8 小时。
// parseTs 统一将「无时区信息」的字符串按 UTC 解释，确保显示为正确的本地时间。

const TZ_RE = /[zZ]$|[+-]\d{2}:?\d{2}$/;

export function parseTs(value: string | null | undefined): Date | null {
  if (!value) return null;
  let s = value.trim();
  if (!s) return null;
  // 无时区信息：按 UTC 处理（空格转 T，补 Z）。
  if (!TZ_RE.test(s)) {
    s = s.replace(' ', 'T');
    s = s + 'Z';
  }
  const d = new Date(s);
  return isNaN(d.getTime()) ? null : d;
}

function sameDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear()
    && a.getMonth() === b.getMonth()
    && a.getDate() === b.getDate();
}

// 仅时分，本地时区：10:00
export function fmtTime(value: string | null | undefined): string {
  const d = parseTs(value);
  if (!d) return '';
  return d.toLocaleTimeString('zh', { hour: '2-digit', minute: '2-digit' });
}

// 消息气泡时间：今天只显示时分，否则带月日；非今年带年份。
export function fmtMsgTime(value: string | null | undefined): string {
  const d = parseTs(value);
  if (!d) return '';
  const now = new Date();
  const time = d.toLocaleTimeString('zh', { hour: '2-digit', minute: '2-digit' });
  if (sameDay(d, now)) return time;
  const opts: Intl.DateTimeFormatOptions = d.getFullYear() === now.getFullYear()
    ? { month: 'numeric', day: 'numeric' }
    : { year: 'numeric', month: 'numeric', day: 'numeric' };
  return `${d.toLocaleDateString('zh', opts)} ${time}`;
}

// 简短日期时间：月日 + 时分（本地时区），用于列表项。
export function fmtShort(value: string | null | undefined): string {
  const d = parseTs(value);
  if (!d) return '';
  return d.toLocaleString('zh', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' });
}

// 完整日期时间，用于 title 悬浮提示。
export function fmtFull(value: string | null | undefined): string {
  const d = parseTs(value);
  if (!d) return '';
  return d.toLocaleString('zh');
}

// 会话列表时间：今天显示时分，否则显示月日。
export function fmtListTime(value: string | null | undefined): string {
  const d = parseTs(value);
  if (!d) return '';
  const now = new Date();
  if (sameDay(d, now)) {
    return d.toLocaleTimeString('zh', { hour: '2-digit', minute: '2-digit' });
  }
  return d.toLocaleDateString('zh', { month: 'numeric', day: 'numeric' });
}
