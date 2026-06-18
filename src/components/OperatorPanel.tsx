import React, { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import Icon from './Icon';
import { MeAvatar } from './Avatar';
import { useOperator, saveOperator, DEFAULT_OPERATOR, type OperatorProfile } from '../operator';
import {
  listNotifications, markNotificationRead, markAllNotificationsRead,
  type AppNotification, type NotificationCategory,
} from '../services';

// 强调色预设：只取主题语义色 + ember，避免引入自由取色破坏主题一致性。
const ACCENT_PRESETS: { label: string; value: string }[] = [
  { label: '默认', value: '' },
  { label: 'ember', value: 'var(--ember)' },
  { label: 'green', value: 'var(--green)' },
  { label: 'blue', value: 'var(--blue)' },
  { label: 'violet', value: 'var(--violet)' },
  { label: 'amber', value: 'var(--amber)' },
];

// 通知分类 → 展示元信息（语义色仅表达状态，不作装饰）。
const CAT_META: Record<NotificationCategory, { label: string; color: string; icon: string }> = {
  intervene: { label: '需介入', color: 'var(--red)', icon: 'alert' },
  progress:  { label: '进度',   color: 'var(--amber)', icon: 'clock' },
  result:    { label: '结果',   color: 'var(--green)', icon: 'check' },
  intake:    { label: '录入',   color: 'var(--blue)', icon: 'inbox' },
};

const FILTERS: { key: 'all' | NotificationCategory; label: string }[] = [
  { key: 'all', label: '全部' },
  { key: 'intervene', label: '需介入' },
  { key: 'progress', label: '进度' },
  { key: 'result', label: '结果' },
  { key: 'intake', label: '录入' },
];

function relativeTime(iso: string): string {
  // 后端 datetime('now') 是 UTC，无时区后缀；补 'Z' 让浏览器按 UTC 解析。
  const t = Date.parse(iso.includes('T') ? iso : iso.replace(' ', 'T') + 'Z');
  if (Number.isNaN(t)) return '';
  const diff = Date.now() - t;
  const s = Math.max(0, Math.round(diff / 1000));
  if (s < 60) return '刚刚';
  const m = Math.round(s / 60);
  if (m < 60) return `${m} 分钟前`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} 小时前`;
  const d = Math.round(h / 24);
  return `${d} 天前`;
}

interface Props {
  onClose: () => void;
  onNavigate: (page: 'audit' | 'delivery' | 'chat', opts?: { openLedger?: boolean }) => void;
  /** 标记已读后通知父级刷新角标。 */
  onUnreadChange: () => void;
}

export default function OperatorPanel({ onClose, onNavigate, onUnreadChange }: Props) {
  const current = useOperator();
  const [draft, setDraft] = useState<OperatorProfile>(current);
  const [saving, setSaving] = useState(false);
  const [items, setItems] = useState<AppNotification[]>([]);
  const [filter, setFilter] = useState<'all' | NotificationCategory>('all');
  const [editing, setEditing] = useState(false);

  const dirty = useMemo(() =>
    draft.display_name !== current.display_name ||
    draft.avatar !== current.avatar ||
    draft.accent_color !== current.accent_color ||
    draft.role !== current.role,
    [draft, current]);

  const refresh = () => {
    listNotifications(60).then(r => setItems(r.items)).catch(() => setItems([]));
  };

  useEffect(() => { refresh(); }, []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const save = async () => {
    setSaving(true);
    try {
      await saveOperator(draft);
      setEditing(false);
    } catch { /* 保留编辑态 */ }
    finally { setSaving(false); }
  };

  const openNotification = async (n: AppNotification) => {
    if (!n.read) {
      try { await markNotificationRead(n.id); } catch { /* ignore */ }
      setItems(prev => prev.map(x => x.id === n.id ? { ...x, read: true } : x));
      onUnreadChange();
    }
    if (n.link_page && (n.link_page === 'audit' || n.link_page === 'delivery' || n.link_page === 'chat')) {
      // 新需求录入：跳到功能审计后自动打开「全量需求总账」弹窗。
      onNavigate(n.link_page, { openLedger: n.kind === 'issue_created' });
      onClose();
    }
  };

  const markAll = async () => {
    try { await markAllNotificationsRead(); } catch { /* ignore */ }
    setItems(prev => prev.map(x => ({ ...x, read: true })));
    onUnreadChange();
  };

  const filtered = filter === 'all' ? items : items.filter(n => n.category === filter);
  const unread = items.filter(n => !n.read).length;

  return createPortal(
    <>
      {/* 透明点击捕获层：popover（菜单语义，非模态）点击外部关闭；不画遮罩。 */}
      <div
        onMouseDown={onClose}
        style={{ position: 'fixed', inset: 'var(--win-gutter, 0)', zIndex: 900 }}
      />
      <div
        className="op-panel rise"
        onMouseDown={e => e.stopPropagation()}
        style={{
          position: 'fixed',
          left: 'calc(var(--win-gutter, 0px) + var(--rail-w) + 8px)',
          bottom: 'calc(var(--win-gutter, 0px) + 14px)',
          width: 340, maxHeight: '74vh', zIndex: 901,
          background: 'var(--bg-2)', border: '1px solid var(--border-strong)',
          borderRadius: 14, boxShadow: 'var(--shadow-lg)',
          display: 'flex', flexDirection: 'column', overflow: 'hidden',
        }}
      >
        {/* ── 身份卡 ───────────────────────────────────────── */}
        <div style={{ padding: '14px 16px', borderBottom: '1px solid var(--border)' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <MeAvatar size={44} />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontWeight: 700, fontSize: 'var(--text-title)', fontFamily: 'var(--font-display)' }}>
                {current.display_name}
              </div>
              {current.role && (
                <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)', letterSpacing: '.04em', marginTop: 1 }}>
                  {current.role}
                </div>
              )}
            </div>
            <button
              className="icon-btn"
              title={editing ? '收起编辑' : '编辑身份'}
              onClick={() => { setDraft(current); setEditing(v => !v); }}
            >
              <Icon name={editing ? 'chevron' : 'edit'} size={14} />
            </button>
          </div>

          {editing && (
            <div style={{ marginTop: 12, display: 'flex', flexDirection: 'column', gap: 10 }}>
              <div className="field">
                <label>显示名</label>
                <input
                  value={draft.display_name}
                  onChange={e => setDraft(d => ({ ...d, display_name: e.target.value }))}
                  placeholder={DEFAULT_OPERATOR.display_name} maxLength={24}
                />
              </div>
              <div style={{ display: 'flex', gap: 10 }}>
                <div className="field" style={{ width: 96 }}>
                  <label>头像</label>
                  <input
                    value={draft.avatar}
                    onChange={e => setDraft(d => ({ ...d, avatar: e.target.value }))}
                    placeholder="管 / 🙂" maxLength={2}
                  />
                </div>
                <div className="field" style={{ flex: 1 }}>
                  <label>头衔</label>
                  <input
                    value={draft.role}
                    onChange={e => setDraft(d => ({ ...d, role: e.target.value }))}
                    placeholder="操作者" maxLength={16}
                  />
                </div>
              </div>
              <div className="field">
                <label>强调色</label>
                <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                  {ACCENT_PRESETS.map(p => {
                    const on = draft.accent_color === p.value;
                    return (
                      <button
                        key={p.label}
                        title={p.label}
                        onClick={() => setDraft(d => ({ ...d, accent_color: p.value }))}
                        style={{
                          width: 26, height: 26, borderRadius: 8, cursor: 'pointer',
                          background: p.value || 'var(--me-avatar-bg)',
                          border: on ? '2px solid var(--ember)' : '1px solid var(--border-strong)',
                          boxShadow: on ? '0 0 0 3px var(--ember-tint)' : 'none',
                        }}
                      />
                    );
                  })}
                </div>
              </div>
              <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
                <button className="btn btn-sm btn-ghost" onClick={() => { setDraft(current); setEditing(false); }}>取消</button>
                <button className="btn btn-sm btn-primary" onClick={save} disabled={!dirty || saving}>
                  <Icon name="check" size={13} />{saving ? '保存中…' : '保存'}
                </button>
              </div>
            </div>
          )}
        </div>

        {/* ── 活动收件箱 ────────────────────────────────────── */}
        <div style={{ padding: '12px 16px 8px', display: 'flex', alignItems: 'center', gap: 8 }}>
          <Icon name="bell" size={14} style={{ color: 'var(--text-3)' }} />
          <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', letterSpacing: '.12em', textTransform: 'uppercase', color: 'var(--text-3)', flex: 1 }}>
            活动 {unread > 0 && <span style={{ color: 'var(--ember)' }}>· {unread} 未读</span>}
          </span>
          {unread > 0 && (
            <button className="btn btn-sm btn-ghost" onClick={markAll} title="全部标记已读">
              <Icon name="check" size={13} />全部已读
            </button>
          )}
        </div>

        <div style={{ padding: '0 16px 8px', display: 'flex', gap: 5, flexWrap: 'wrap' }}>
          {FILTERS.map(f => (
            <button
              key={f.key}
              className={'filter-chip' + (filter === f.key ? ' on' : '')}
              onClick={() => setFilter(f.key)}
            >
              {f.label}
            </button>
          ))}
        </div>

        <div style={{ overflowY: 'auto', padding: '0 8px 10px', flex: 1 }}>
          {filtered.length === 0 ? (
            <div className="empty" style={{ padding: '24px 12px', textAlign: 'center', color: 'var(--text-faint)', fontSize: 'var(--text-label)' }}>
              <Icon name="inbox" size={22} style={{ opacity: .5 }} />
              <div style={{ marginTop: 6 }}>暂无活动</div>
            </div>
          ) : filtered.map(n => {
            const meta = CAT_META[n.category];
            const clickable = !!n.link_page;
            return (
              <div
                key={n.id}
                onClick={() => openNotification(n)}
                style={{
                  display: 'flex', gap: 10, padding: '9px 8px', borderRadius: 9,
                  cursor: clickable ? 'pointer' : 'default',
                  background: n.read ? 'transparent' : 'var(--ember-tint)',
                  alignItems: 'flex-start',
                }}
              >
                <span style={{
                  marginTop: 2, width: 22, height: 22, borderRadius: 6, flexShrink: 0,
                  display: 'grid', placeItems: 'center',
                  background: 'color-mix(in srgb, ' + meta.color + ' 16%, transparent)',
                  color: meta.color,
                }}>
                  <Icon name={meta.icon} size={12} />
                </span>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ display: 'flex', alignItems: 'baseline', gap: 6 }}>
                    <span style={{ fontSize: 'var(--text-control)', fontWeight: n.read ? 500 : 700, color: 'var(--text)', flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {n.title}
                    </span>
                    <span style={{ fontSize: 'var(--text-micro)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', flexShrink: 0 }}>
                      {relativeTime(n.created_at)}
                    </span>
                  </div>
                  {n.body && (
                    <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {n.body}
                    </div>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </>,
    document.querySelector('.os-window') ?? document.body,
  );
}
