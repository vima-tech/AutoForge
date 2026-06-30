import React, { useState, useEffect } from 'react';
import Icon from './Icon';
import { getIssueLifecycle, type LifecycleEvent } from '../services';

interface Props {
  /** 该 CR 关联的需求 id（生命周期以需求为主线聚合）。 */
  issueId: string;
}

// 事件类别 → 图标 + 语义色。
const KIND_META: Record<string, { icon: string; color: string }> = {
  created:     { icon: 'inbox',   color: 'var(--text-3)' },
  analyzed:    { icon: 'search',  color: 'var(--violet)' },
  decision:    { icon: 'check',   color: 'var(--ember)' },
  cr_created:  { icon: 'layers',  color: 'var(--blue)' },
  cr_approved: { icon: 'check',   color: 'var(--green)' },
  worktree:    { icon: 'code',    color: 'var(--amber)' },
  merged:      { icon: 'merge',   color: 'var(--green)' },
};

/**
 * 需求溯源时间线：把一条需求从录入到合并散落各表的关键节点聚合成竖向时间线。
 * 折叠面板，默认收起，按需展开拉取（避免每次切 CR 都请求）。
 */
export default function LifecyclePanel({ issueId }: Props) {
  const [open, setOpen] = useState(false);
  const [events, setEvents] = useState<LifecycleEvent[] | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => { setEvents(null); setOpen(false); }, [issueId]);

  useEffect(() => {
    if (!open || events || !issueId) return;
    let alive = true;
    setLoading(true);
    getIssueLifecycle(issueId)
      .then(e => { if (alive) setEvents(e); })
      .catch(() => { if (alive) setEvents([]); })
      .finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [open, events, issueId]);

  if (!issueId) return null;

  return (
    <div className="panel" style={{ margin: '0 clamp(8px, 1.4vw, 24px) 12px', overflow: 'hidden' }}>
      <button
        className="panel-head"
        style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '11px 14px', width: '100%', background: 'none', border: 'none', cursor: 'pointer', textAlign: 'left' }}
        onClick={() => setOpen(o => !o)}
      >
        <Icon name="layers" size={16} style={{ color: 'var(--ember)' }} />
        <span style={{ fontWeight: 700, fontSize: 'var(--text-title)' }}>溯源时间线</span>
        <Icon name="chevron" size={14} style={{ marginLeft: 'auto', color: 'var(--text-3)', transform: open ? 'none' : 'rotate(-90deg)', transition: 'transform .15s' }} />
      </button>

      {open && (
        <div style={{ padding: '12px 16px' }}>
          {loading && <div style={{ color: 'var(--text-3)', fontSize: 'var(--text-control)' }}>加载中…</div>}
          {events && events.length === 0 && !loading && (
            <div style={{ color: 'var(--text-faint)', fontSize: 'var(--text-control)' }}>暂无可追溯的节点。</div>
          )}
          {events && events.length > 0 && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
              {events.map((e, i) => {
                const m = KIND_META[e.kind] ?? { icon: 'layers', color: 'var(--text-3)' };
                return (
                  <div key={i} style={{ display: 'flex', gap: 10, alignItems: 'flex-start' }}>
                    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', alignSelf: 'stretch' }}>
                      <span style={{ display: 'grid', placeItems: 'center', width: 24, height: 24, borderRadius: 'var(--radius-sm)', background: 'var(--bg-3)', color: m.color, flex: 'none' }}>
                        <Icon name={m.icon} size={13} />
                      </span>
                      {i < events.length - 1 && <span style={{ flex: 1, width: 2, background: 'var(--border)', minHeight: 10 }} />}
                    </div>
                    <div style={{ paddingBottom: 12, minWidth: 0 }}>
                      <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, flexWrap: 'wrap' }}>
                        <span style={{ fontWeight: 600, fontSize: 'var(--text-control)' }}>{e.label}</span>
                        {e.at && <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-faint)' }}>{e.at}</span>}
                      </div>
                      {e.detail && <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', marginTop: 2, wordBreak: 'break-word' }}>{e.detail}</div>}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
