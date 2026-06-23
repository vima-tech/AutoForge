import React, { useState, useEffect, useCallback } from 'react';
import Icon from './Icon';
import { generateChangeSummary, type ChangeSummary, type SummaryFile, type SensitiveTag } from '../services';

// 文件改动类型 → 图标 + 语义色 + 文案。
const CHANGE_META: Record<SummaryFile['change'], { icon: string; color: string; label: string }> = {
  added:    { icon: 'plus',    color: 'var(--green)',  label: '新增' },
  modified: { icon: 'edit',    color: 'var(--amber)',  label: '修改' },
  deleted:  { icon: 'trash',   color: 'var(--red)',    label: '删除' },
  renamed:  { icon: 'merge',   color: 'var(--blue)',   label: '重命名' },
};

// 敏感标签 kind → chip 语义变体 + 图标（关键词/LLM 双源，未知 kind 兜底）。
const SENSITIVE_META: Record<string, { chip: string; icon: string }> = {
  database:   { chip: 'amber',  icon: 'layers' },
  migration:  { chip: 'red',    icon: 'layers' },
  api:        { chip: 'blue',   icon: 'code' },
  permission: { chip: 'violet', icon: 'shield' },
  secret:     { chip: 'red',    icon: 'key' },
};
const sensitiveMeta = (t: SensitiveTag) => SENSITIVE_META[t.kind] ?? { chip: 'amber', icon: 'alert' };

interface Props {
  crId: string;
  /** 当前 CR 是否有可摘要的代码改动（无改动/失败/冲突态传 false，跳过请求）。 */
  enabled: boolean;
}

/**
 * AI 变更摘要卡片：基于 CR 的 diff 实时生成结构化摘要。
 * 三区块：业务意图分类 · 修改文件清单 · 敏感模块高亮标签。
 * loading/error/degraded/empty 全状态降级，绝不阻塞 Diff 渲染。
 */
export default function ChangeSummaryCard({ crId, enabled }: Props) {
  const [summary, setSummary] = useState<ChangeSummary | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);
  const [collapsed, setCollapsed] = useState(false);

  // force=false：走后端缓存（按 cr_id + diff 哈希），切换需求/CR 命中即返回、不重跑 LLM。
  // force=true：用户点「重新生成」时跳过缓存强制重算。
  const load = useCallback((force = false) => {
    if (!enabled) { setSummary(null); return; }
    let alive = true;
    setLoading(true);
    setError(false);
    setSummary(null);
    generateChangeSummary(crId, force)
      .then(s => { if (alive) setSummary(s); })
      .catch(() => { if (alive) setError(true); })
      .finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [crId, enabled]);

  // crId / enabled 变化时自动请求（命中缓存不重算）；返回的 cleanup 取消上一请求的回写，避免竞态。
  useEffect(() => {
    const cleanup = load();
    return cleanup;
  }, [load]);

  if (!enabled) return null;
  // 空摘要（无 diff）：不占位，交由下方报告/diff 区自行提示。
  if (summary?.status === 'empty') return null;

  return (
    <div className="panel" style={{ margin: '0 clamp(8px, 1.4vw, 24px) 12px', overflow: 'hidden' }}>
      {/* 卡头：标题 + AI 生成提示 + 重新生成 / 折叠 */}
      <div
        className="panel-head"
        style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '11px 14px', cursor: 'default' }}
      >
        <Icon name="zap" size={16} style={{ color: 'var(--ember)' }} />
        <span style={{ fontWeight: 700, fontSize: 'var(--text-title)' }}>AI 变更摘要</span>
        <span
          className="chip"
          style={{ fontSize: 'var(--text-micro)', fontFamily: 'var(--font-mono)', color: 'var(--text-3)' }}
          title="由 AI 基于 diff 生成，仅供参考，请独立判断"
        >
          AI 生成
        </span>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 4 }}>
          <button className="icon-btn" title="重新生成摘要" onClick={() => load(true)} disabled={loading}>
            <Icon name="refresh" size={14} />
          </button>
          <button
            className="icon-btn"
            title={collapsed ? '展开' : '收起'}
            onClick={() => setCollapsed(c => !c)}
          >
            <Icon name={collapsed ? 'eye' : 'eye-off'} size={14} />
          </button>
        </div>
      </div>

      {!collapsed && (
        <div style={{ padding: '12px 14px' }}>
          {loading ? (
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, color: 'var(--text-3)', fontSize: 'var(--text-control)' }}>
              <span className="dot amber" />
              正在分析变更并生成摘要…
            </div>
          ) : error ? (
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, color: 'var(--text-3)', fontSize: 'var(--text-control)' }}>
              <Icon name="alert" size={15} style={{ color: 'var(--amber)' }} />
              摘要生成失败。
              <button className="btn btn-sm btn-ghost" onClick={() => load()}>
                <Icon name="refresh" size={13} />重试
              </button>
            </div>
          ) : summary ? (
            <>
              {/* 降级提示：LLM 不可用时仅余确定性部分 */}
              {summary.status === 'degraded' && summary.note && (
                <div
                  style={{
                    display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12,
                    padding: '8px 10px', borderRadius: 8,
                    background: 'var(--bg-3)', border: '1px solid var(--border-strong)',
                    color: 'var(--text-2)', fontSize: 'var(--text-label)',
                  }}
                >
                  <Icon name="alert" size={14} style={{ color: 'var(--amber)', flexShrink: 0 }} />
                  {summary.note}
                </div>
              )}

              {/* 概述 */}
              {summary.overview && (
                <p style={{ margin: '0 0 12px', fontSize: 'var(--text-control)', color: 'var(--text)', lineHeight: 'var(--leading-relaxed)' }}>
                  {summary.overview}
                </p>
              )}

              {/* 敏感模块高亮标签 */}
              {summary.sensitive.length > 0 && (
                <div style={{ marginBottom: 12 }}>
                  <SectionLabel icon="shield" text="敏感模块" />
                  <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
                    {summary.sensitive.map((t, i) => {
                      const m = sensitiveMeta(t);
                      return (
                        <span
                          key={i}
                          className={'chip ' + m.chip}
                          style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}
                          title={t.detail || t.label}
                        >
                          <Icon name={m.icon} size={12} />{t.label}
                        </span>
                      );
                    })}
                  </div>
                </div>
              )}

              {/* 业务意图分类 */}
              {summary.intents.length > 0 && (
                <div style={{ marginBottom: 12 }}>
                  <SectionLabel icon="brain" text="业务意图" />
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                    {summary.intents.map((g, i) => (
                      <div key={i} style={{ display: 'flex', gap: 8, alignItems: 'baseline' }}>
                        <span className="chip ember" style={{ flexShrink: 0, fontSize: 'var(--text-micro)' }}>{g.kind || '其他'}</span>
                        <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-2)', lineHeight: 'var(--leading-normal)' }}>{g.detail}</span>
                      </div>
                    ))}
                  </div>
                </div>
              )}

              {/* 修改文件清单 */}
              {summary.files.length > 0 && (
                <div>
                  <SectionLabel icon="file" text={`修改文件 · ${summary.files.length}`} />
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                    {summary.files.map((f, i) => {
                      const m = CHANGE_META[f.change] ?? CHANGE_META.modified;
                      return (
                        <div
                          key={i}
                          style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 'var(--text-label)' }}
                        >
                          <Icon name={m.icon} size={13} style={{ color: m.color, flexShrink: 0 }} />
                          <span title={f.change} style={{ flexShrink: 0, width: 38, color: m.color, fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', lineHeight: 'var(--leading-normal)' }}>{m.label}</span>
                          {/* 路径与说明纵向堆叠，避免挤在一行导致路径断词、说明错位 */}
                          <div style={{ minWidth: 0, display: 'flex', flexDirection: 'column', gap: 2 }}>
                            <span style={{ fontFamily: 'var(--font-mono)', color: 'var(--text)', overflowWrap: 'anywhere', lineHeight: 'var(--leading-normal)' }}>{f.path}</span>
                            {f.note && <span style={{ color: 'var(--text-3)', lineHeight: 'var(--leading-normal)' }}>{f.note}</span>}
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </div>
              )}
            </>
          ) : null}
        </div>
      )}
    </div>
  );
}

function SectionLabel({ icon, text }: { icon: string; text: string }) {
  return (
    <div
      style={{
        display: 'flex', alignItems: 'center', gap: 6, marginBottom: 7,
        fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)',
        letterSpacing: '.12em', textTransform: 'uppercase', color: 'var(--text-3)',
      }}
    >
      <Icon name={icon} size={13} />{text}
    </div>
  );
}
