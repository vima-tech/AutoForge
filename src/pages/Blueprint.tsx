import React, { useEffect, useRef, useState } from 'react';
import Icon from '../components/Icon';
import BlueprintStudio, { DISPLAY_STATUS } from '../components/BlueprintStudio';
import { listProjects, listBlueprintDrafts, deleteBlueprintDraft, type Project, type BlueprintDraftSummary } from '../services';

/**
 * 孵化台 —— 把多条「大需求改动」梳理成 PRD/规格/任务，并一键「编码开发」直接实现。
 * 列表视图（多需求 + 状态）↔ 详情视图（BlueprintStudio 工作台）。每条大需求 = 一个草稿；
 * 编码开发后落为 issue + CR，状态（编码中/待审核/已实现）回链到卡片。
 */
export default function BlueprintPage({ target, onTargetConsumed, onOpenAudit }: {
  target?: { projectId: string } | null;
  onTargetConsumed?: () => void;
  onOpenAudit?: (projectId: string, issueId: string) => void;
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [loadingProjects, setLoadingProjects] = useState(true);
  const [drafts, setDrafts] = useState<BlueprintDraftSummary[]>([]);
  const [loadingDrafts, setLoadingDrafts] = useState(false);
  const [selectedDraftId, setSelectedDraftId] = useState<string | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [projOpen, setProjOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<BlueprintDraftSummary | null>(null);
  const [deleting, setDeleting] = useState(false);
  const projRef = useRef<HTMLDivElement>(null);

  const active = projects.find(p => p.id === activeId) ?? null;
  const inDetail = isNew || !!selectedDraftId;

  const loadProjects = async () => {
    setLoadingProjects(true);
    try {
      const ps = await listProjects();
      setProjects(ps);
      setActiveId(prev => prev && ps.some(p => p.id === prev) ? prev : (ps[0]?.id ?? null));
    } catch { /* keep empty */ }
    finally { setLoadingProjects(false); }
  };
  const loadDrafts = async (pid: string) => {
    setLoadingDrafts(true);
    try { setDrafts(await listBlueprintDrafts(pid)); }
    catch { setDrafts([]); }
    finally { setLoadingDrafts(false); }
  };

  useEffect(() => { void loadProjects(); }, []);
  useEffect(() => { if (activeId) void loadDrafts(activeId); }, [activeId]);

  // 跨页深链：Projects「项目蓝图」按钮 → 选中该项目并回到列表。
  useEffect(() => {
    if (!target) return;
    setActiveId(target.projectId);
    setSelectedDraftId(null); setIsNew(false);
    onTargetConsumed?.();
  }, [target]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!projOpen) return;
    const h = (e: PointerEvent) => { if (e.target instanceof Node && projRef.current?.contains(e.target)) return; setProjOpen(false); };
    document.addEventListener('pointerdown', h);
    return () => document.removeEventListener('pointerdown', h);
  }, [projOpen]);

  const backToList = () => { setSelectedDraftId(null); setIsNew(false); if (activeId) void loadDrafts(activeId); };

  const confirmDelete = async () => {
    if (!deleteTarget) return;
    setDeleting(true);
    try { await deleteBlueprintDraft(deleteTarget.id); setDeleteTarget(null); if (activeId) await loadDrafts(activeId); }
    catch (e) { alert(String(e)); }
    finally { setDeleting(false); }
  };

  useEffect(() => {
    if (!deleteTarget) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setDeleteTarget(null); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [deleteTarget]);

  // ── 详情视图：工作台 ──
  if (inDetail && active) {
    return (
      <BlueprintStudio
        key={selectedDraftId ?? 'new'}
        project={active}
        draftId={selectedDraftId}
        isNew={isNew}
        onBack={backToList}
        onChanged={(newId) => { if (newId) { setSelectedDraftId(newId); setIsNew(false); } if (activeId) void loadDrafts(activeId); }}
        onOpenAudit={onOpenAudit}
      />
    );
  }

  // ── 列表视图 ──
  return (
    <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
      {/* 头栏：孵化台标识 + 项目选择器 + 新建需求 */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 14, padding: '14px 20px 12px', flexShrink: 0, borderBottom: '1px solid var(--border)' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
          <Icon name="layers" size={18} style={{ color: 'var(--ember)' }} />
          <span style={{ fontFamily: 'var(--font-display)', fontWeight: 800, fontSize: 'var(--text-heading)' }}>孵化台</span>
          <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', letterSpacing: '.14em', textTransform: 'uppercase', color: 'var(--text-faint)' }}>
            从念头到可交付
          </span>
        </div>
        <div style={{ position: 'relative', minWidth: 240, maxWidth: 340 }} ref={projRef}>
          {active ? (
            <div className="proj-select" onClick={() => setProjOpen(o => !o)} style={{ cursor: 'pointer' }}>
              <div className="proj-logo" style={{ background: '#e8772e' }}>{active.name[0]}</div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div className="proj-name">{active.name}</div>
                <div className="proj-meta">{active.description || active.slug}</div>
              </div>
              <Icon name="chevDown" size={16} style={{ color: 'var(--text-3)', transition: 'transform .15s', transform: projOpen ? 'rotate(180deg)' : 'none' }} />
            </div>
          ) : (
            <div className="empty-compact" style={{ padding: '8px 10px' }}>{loadingProjects ? '载入项目…' : '暂无项目'}</div>
          )}
          {projOpen && (
            <div className="mention-pop" style={{ left: 0, right: 0, top: 'calc(100% + 6px)', bottom: 'auto', width: '100%', marginBottom: 0, maxHeight: 360, overflowY: 'auto' }}>
              {projects.map(p => (
                <div key={p.id} className="mention-row" onClick={() => { setActiveId(p.id); setProjOpen(false); }}>
                  <div className="proj-logo" style={{ background: '#e8772e', width: 28, height: 28, fontSize: 'var(--text-label)', borderRadius: 8 }}>{p.name[0]}</div>
                  <div style={{ minWidth: 0, flex: 1 }}>
                    <div className="nm" style={{ display: 'flex', alignItems: 'center', gap: 5 }}>{p.name}{p.id === activeId && <span style={{ width: 5, height: 5, borderRadius: '50%', background: 'var(--ember)', display: 'inline-block' }} />}</div>
                    <div className="rl">{p.description || p.slug}</div>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
        <div style={{ flex: 1 }} />
        <button className="btn btn-primary" disabled={!active} onClick={() => { setIsNew(true); setSelectedDraftId(null); }}>
          <Icon name="plus" size={14} />新建需求改动
        </button>
      </div>

      {/* 需求卡片列表 */}
      <div style={{ flex: 1, overflowY: 'auto', padding: '18px 20px 28px' }}>
        {!active ? (
          <div className="empty" style={{ height: '60%' }}><Icon name="layers" size={40} style={{ opacity: .3 }} /><div>先在「项目管理」添加一个项目</div></div>
        ) : loadingDrafts ? (
          <div className="empty" style={{ height: '40%' }}><Icon name="refresh" size={26} style={{ opacity: .4 }} /><div>载入需求…</div></div>
        ) : drafts.length === 0 ? (
          <div className="empty" style={{ height: '50%', gap: 12 }}>
            <Icon name="layers" size={40} style={{ opacity: .3 }} />
            <div>还没有大需求改动。点右上角「新建需求改动」，把一段大需求炼成 PRD / 规格 / 任务并一键编码。</div>
          </div>
        ) : (
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(300px, 1fr))', gap: 14 }}>
            {drafts.map(d => {
              const st = DISPLAY_STATUS[d.display_status] ?? DISPLAY_STATUS.drafting;
              return (
                <div key={d.id} className="panel" style={{ padding: '14px 16px', display: 'flex', flexDirection: 'column', gap: 10, cursor: 'pointer' }}
                  onClick={() => { setSelectedDraftId(d.id); setIsNew(false); }}>
                  <div style={{ display: 'flex', alignItems: 'flex-start', gap: 8 }}>
                    <div style={{ flex: 1, minWidth: 0, fontWeight: 600, fontSize: 'var(--text-title)', lineHeight: 'var(--leading-snug, 1.3)' }}>{d.title || '未命名需求'}</div>
                    <span className={'chip ' + st.chip} style={{ fontSize: 'var(--text-micro)', flexShrink: 0 }}>{st.label}</span>
                  </div>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 12, fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>
                    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}><Icon name="layers" size={12} />{d.spec_count} 规格</span>
                    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}><Icon name="list" size={12} />{d.task_count} 任务</span>
                    <div style={{ flex: 1 }} />
                    {d.display_status === 'drafting' && (
                      <button className="icon-btn" style={{ width: 26, height: 26, color: 'var(--red)' }}
                        onClick={e => { e.stopPropagation(); setDeleteTarget(d); }} title="删除该需求草稿">
                        <Icon name="trash" size={12} />
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* 删除二次确认（关闭只走 ✕ / Esc，不点遮罩关闭，符合 DESIGN.md）*/}
      {deleteTarget && (
        <div style={{ position: 'fixed', inset: 'var(--win-gutter, 0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(2px)', display: 'grid', placeItems: 'center', zIndex: 1000 }}>
          <div className="rise" style={{ width: 'min(440px, 92%)', background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 16, boxShadow: 'var(--shadow-lg)', padding: '18px 20px' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 12 }}>
              <div style={{ width: 30, height: 30, borderRadius: 8, background: 'rgba(219,90,64,.14)', display: 'grid', placeItems: 'center', flexShrink: 0 }}>
                <Icon name="trash" size={15} style={{ color: 'var(--red)' }} />
              </div>
              <div style={{ flex: 1, fontWeight: 600, fontSize: 'var(--text-body)' }}>删除需求改动</div>
              <button className="icon-btn" onClick={() => setDeleteTarget(null)} title="取消"><Icon name="x" size={15} /></button>
            </div>
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-2)', lineHeight: 'var(--leading-relaxed)' }}>
              确定删除「<b style={{ color: 'var(--text)' }}>{deleteTarget.title || '未命名需求'}</b>」？其 PRD / 规格 / 任务草稿与对话历史将一并删除，且不可恢复。
            </div>
            <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 10, marginTop: 18 }}>
              <button className="btn btn-sm btn-ghost" onClick={() => setDeleteTarget(null)} disabled={deleting}>取消</button>
              <button className="btn btn-sm btn-danger" onClick={confirmDelete} disabled={deleting}>
                <Icon name="trash" size={13} />{deleting ? '删除中…' : '删除'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
