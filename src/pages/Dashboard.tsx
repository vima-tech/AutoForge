import React, { useState, useEffect, useCallback } from 'react';
import { listen } from '@tauri-apps/api/event';
import Icon from '../components/Icon';
import Select from '../components/Select';
import IntakePanel from '../components/IntakePanel';
import { getPipelineStats, listActiveProjects, listIssues, type PipelineStats, type Project, type Issue } from '../services';

const SEV_COLOR: Record<string, string> = {
  critical: 'red', high: 'amber', medium: 'blue', low: 'green',
  Bug: 'red', Feature: 'ember', Improvement: 'blue', Debt: 'violet',
};

// ── Submit Issue Modal ────────────────────────────────────────────────────────
// Hosts the shared IntakePanel (手动提交 / GitHub / 代码扫描 / 批量导入) so the
// homepage entry stays consistent with the project-level「需求入口」in Audit.
function SubmitIssueModal({ projects, onClose }: { projects: Project[]; onClose: () => void }) {
  const [projectId, setProjectId] = useState(projects[0]?.id ?? '');

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 60 }}>
      <div style={{ width: 720, maxHeight: 'min(800px, calc(100vh - 32px))', background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', overflow: 'hidden', display: 'flex', flexDirection: 'column' }} onClick={e => e.stopPropagation()}>
        <div style={{ padding: '18px 20px 14px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', gap: 12 }}>
          <div className="eyebrow" style={{ fontSize: 'var(--text-section)', flexShrink: 0 }}><span className="cn">需求入口</span></div>
          <div style={{ marginLeft: 'auto', minWidth: 200 }}>
            <Select value={projectId} onChange={setProjectId}
              options={projects.map(p => ({ value: p.id, label: p.name }))}
              placeholder="选择项目" />
          </div>
          <button className="icon-btn" onClick={onClose}><Icon name="x" size={18} /></button>
        </div>
        <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
          {projectId
            ? <IntakePanel key={projectId} projectId={projectId} />
            : <div className="empty" style={{ flex: 1 }}><Icon name="inbox" /><div>请先在「项目管理」添加项目</div></div>}
        </div>
      </div>
    </div>
  );
}

// ── Dashboard ─────────────────────────────────────────────────────────────────
export default function Dashboard({ onOpenInAudit }: {
  onOpenInAudit: (target: { projectId: string; issueId: string }) => void;
}) {
  const [stats, setStats] = useState<PipelineStats | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [issues, setIssues] = useState<Issue[]>([]);
  const [showSubmit, setShowSubmit] = useState(false);
  const [carouselIndex, setCarouselIndex] = useState(0);
  const [carouselPaused, setCarouselPaused] = useState(false);

  const loadAll = useCallback(async () => {
    const [s, ps, is] = await Promise.all([getPipelineStats(), listActiveProjects(), listIssues()]);
    setStats(s);
    setProjects(ps);
    setIssues(is.sort((a, b) => {
      const priorityDiff = (b.priority ?? 0) - (a.priority ?? 0);
      if (priorityDiff !== 0) return priorityDiff;
      return new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime();
    }));
  }, []);

  useEffect(() => {
    loadAll();
    let timer: ReturnType<typeof setTimeout> | null = null;
    const debounced = () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => { timer = null; loadAll(); }, 500);
    };
    let unlisten: (() => void) | undefined;
    listen<unknown>('AutoForge://event', debounced).then(fn => { unlisten = fn; });
    return () => {
      if (timer) clearTimeout(timer);
      unlisten?.();
    };
  }, [loadAll]);

  // Build backlog map
  const backlogByProject: Record<string, number> = {};
  issues.forEach(i => { backlogByProject[i.project_id] = (backlogByProject[i.project_id] ?? 0) + 1; });
  const queueIssues = issues.slice(0, 8);
  const activeProjectCount = projects.filter(p => p.status === 'active').length;
  const pendingReview = stats?.pending_review_slots ?? stats?.pending_review_2 ?? 0;
  const pauseThreshold = stats?.pause_threshold ?? 20;
  const pressurePct = Math.min(100, (pendingReview / pauseThreshold) * 100);
  const totalSlotCapacity = stats?.total_slot_capacity ?? Math.max(1, activeProjectCount) * (stats?.max_slots ?? 5);
  const projectSlots = stats?.project_slots ?? [];
  const projectPipelines = stats?.project_pipelines ?? [];
  const carouselCount = Math.max(projectPipelines.length, projectSlots.length);
  const visiblePipeline = projectPipelines.length ? projectPipelines[carouselIndex % projectPipelines.length] : null;
  const visibleSlot = projectSlots.length ? projectSlots[carouselIndex % projectSlots.length] : null;
  const buildPipeline = (project: NonNullable<PipelineStats['project_pipelines']>[number]) => [
    { ic: 'inbox', name: '需求入口',   cnt: project.pending_analysis,  state: project.pending_analysis > 0 ? 'active' : 'done' },
    { ic: 'search', name: '需求分析',  cnt: project.pending_review_1,  state: project.pending_review_1 > 0 ? 'active' : 'done' },
    { ic: 'check', name: '审核 1',     cnt: project.pending_review_1,  state: project.pending_review_1 > 0 ? 'active' : 'done' },
    { ic: 'code', name: 'Claude Code', cnt: project.executing,         state: project.executing > 0 ? 'active' : '' },
    { ic: 'eye', name: '审核 2',       cnt: project.pending_review_2,  state: project.pending_review_2 > 3 ? 'warn' : project.pending_review_2 > 0 ? 'active' : '' },
    { ic: 'merge', name: '合并 dev',   cnt: project.merged,            state: project.merged > 0 ? 'done' : '' },
  ];

  useEffect(() => {
    setCarouselIndex(0);
  }, [carouselCount]);

  useEffect(() => {
    if (carouselPaused) return;
    if (carouselCount <= 1) return;
    const timer = window.setInterval(() => {
      setCarouselIndex(index => (index + 1) % carouselCount);
    }, 3600);
    return () => window.clearInterval(timer);
  }, [carouselCount, carouselPaused]);

  return (
    <div className="dash scroll">
      {showSubmit && <SubmitIssueModal projects={projects} onClose={() => { setShowSubmit(false); loadAll(); }} />}
      <div className="dash-inner rise">
        {/* hero */}
        <div className="dash-hero">
          <div>
            <div className="sec-kicker" style={{ marginBottom: 8 }}>工厂总览 · FACTORY OVERVIEW</div>
            <div className="dash-hello">下午好，管理员</div>
            <div className="dash-sub">
              {stats ? `${stats.stage === 'normal' ? '流水线运行正常' : stats.stage === 'throttled' ? '单线程降速中' : '系统已暂停'} · ${stats.active_slots}/${totalSlotCapacity} 项目槽位占用` : '加载中…'}
            </div>
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <button className="btn btn-primary" onClick={() => setShowSubmit(true)}><Icon name="plus" size={16} />提交需求</button>
          </div>
        </div>

        {/* stats */}
        <div className="stat-grid">
          {[
            { ic: 'box',   color: '#e8772e', val: String(stats?.active_projects ?? activeProjectCount), unit: '', label: '在产项目', delta: `全部 ${projects.length}` },
            { ic: 'inbox', color: '#8b7ad8', val: String(stats?.total_issues ?? issues.length), unit: '', label: '需求总数', delta: '' },
            { ic: 'cpu',   color: '#4f8ed1', val: String(stats?.active_slots ?? '—'), unit: `/${totalSlotCapacity}`, label: '并发槽位占用', delta: `每项目 ${stats?.max_slots ?? 5} 槽 · ${stats?.stage ?? '…'}` },
            { ic: 'clock', color: '#4f9d6b', val: String(stats?.pending_review_2 ?? '—'), unit: '', label: '待审核 (审核2)', delta: '' },
          ].map((s, i) => (
            <div className="stat" key={i}>
              <div className="stat-ic" style={{ background: `color-mix(in oklab, ${s.color} 16%, transparent)`, color: s.color }}><Icon name={s.ic} size={18} /></div>
              <div className="stat-main">
                <div className="stat-label">{s.label}</div>
                <div className="stat-val">{s.val}<span className="u">{s.unit}</span></div>
              </div>
              <div className="stat-delta up">{s.delta || '实时数据'}</div>
            </div>
          ))}
        </div>

        <div className="dash-cols">
          {/* queue */}
          <div className="panel">
            <div className="panel-head">
              <div className="panel-title"><Icon name="inbox" size={17} style={{ color: 'var(--ember)' }} />需求队列 · 优先级排序</div>
              <span className="sec-kicker">显示 {queueIssues.length} / {issues.length} 条</span>
            </div>
            {queueIssues.length === 0
              ? <div className="empty-compact" style={{ padding: '20px 18px' }}>暂无需求</div>
              : queueIssues.map((q, i) => (
              <div className="q-row" key={q.id} onClick={() => onOpenInAudit({ projectId: q.project_id, issueId: q.id })}
                style={{ cursor: 'pointer' }} title="在功能审计中查看">
                <div className="q-pr">{i + 1}</div>
                <div className="q-main">
                  <div className="q-title">{q.title}</div>
                  <div className="q-meta">
                    <span className="req-id" style={{ color: 'var(--text-3)' }}>{q.id.slice(0, 10)}</span>
                    <span className={'chip ' + (SEV_COLOR[q.category] || 'blue')} style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }}>{q.category}</span>
                    <span className={'chip ' + (SEV_COLOR[q.severity] || '')} style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }}>{q.severity}</span>
                    <span>· {projects.find(p => p.id === q.project_id)?.name ?? '—'}</span>
                  </div>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexShrink: 0 }}>
                  <span className={'chip ' + (q.status === 'pending_review_1' ? 'amber' : q.status.includes('review') ? 'amber' : q.status === 'executing' ? 'ember' : '')}>
                    {q.status === 'pending_review_1' ? '待需求审核' : q.status.replace(/_/g, ' ')}
                  </span>
                  <Icon name="chevRight" size={15} style={{ color: 'var(--text-faint)' }} />
                </div>
              </div>
            ))}
          </div>

          {/* slots + backpressure */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
            <div className="panel">
              <div className="panel-head">
                <div className="panel-title"><Icon name="cpu" size={17} style={{ color: 'var(--blue)' }} />并发槽位</div>
                <span className="sec-kicker">总计 {stats?.active_slots ?? 0} / {totalSlotCapacity} · {carouselPaused ? '已暂停' : '自动'} · {projectSlots.length ? `${(carouselIndex % projectSlots.length) + 1}/${projectSlots.length}` : '0/0'}</span>
              </div>
              <div className="project-slots" onMouseEnter={() => setCarouselPaused(true)} onMouseLeave={() => setCarouselPaused(false)}>
                {!visibleSlot
                  ? <div className="empty-state">暂无在产项目槽位</div>
                  : (
                    <div className="project-slot-row carousel-card" key={visibleSlot.project_id}>
                      <div className="project-slot-head">
                        <div>
                          <div className="project-slot-name">{visibleSlot.project_name}</div>
                          <div className="project-slot-meta">执行 {visibleSlot.executing_slots} · 待审核 {visibleSlot.pending_review_slots}</div>
                        </div>
                        <span className="sec-kicker">{visibleSlot.active_slots} / {visibleSlot.max_slots}</span>
                      </div>
                      <div className="slots">
                        {Array.from({ length: visibleSlot.max_slots }).map((_, i) => {
                          const occupant = visibleSlot.occupants[i];
                          const isPending = occupant?.status === 'pending_review_2';
                          return (
                            <div key={i} className={'slot' + (occupant ? ' busy' : '') + (isPending ? ' warn' : '')}>
                              {occupant ? (
                                <>
                                  <span>{occupant.id.slice(0, 10)}</span>
                                  <small>{isPending ? '审核' : '执行'}</small>
                                </>
                              ) : '空闲'}
                            </div>
                          );
                        })}
                      </div>
                    </div>
                  )}
              </div>
            </div>
            <div className="panel">
              <div className="panel-head">
                <div className="panel-title"><Icon name="play" size={15} style={{ color: 'var(--green)' }} />背压状态</div>
                <span className={'chip ' + (stats?.stage === 'paused' ? 'red' : stats?.stage === 'throttled' ? 'amber' : 'green')}>
                  {stats?.stage === 'paused' ? '暂停' : stats?.stage === 'throttled' ? '降速' : '正常'}
                </span>
              </div>
              <div style={{ padding: '14px 18px 18px' }}>
                <div className="bp-bar">
                  <div className="bp-seg" style={{ width: `${pressurePct}%`, background: stats?.stage === 'paused' ? 'var(--red)' : stats?.stage === 'throttled' ? 'var(--amber)' : 'var(--green)' }} />
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>
                  <span>积压 {pendingReview}</span><span>暂停 {pauseThreshold}</span>
                </div>
              </div>
            </div>
          </div>
        </div>

        {/* pipeline */}
        <div className="panel" style={{ marginBottom: 16 }}>
          <div className="panel-head">
            <div className="eyebrow" style={{ fontSize: 'var(--text-body)' }}><span className="en">PIPELINE</span><span className="cn">· 完整流水线</span></div>
            <div className="sec-kicker">{carouselPaused ? '已暂停' : '自动轮播'} · {projectPipelines.length ? `${(carouselIndex % projectPipelines.length) + 1}/${projectPipelines.length}` : '0/0'}</div>
          </div>
          <div className="project-pipelines" onMouseEnter={() => setCarouselPaused(true)} onMouseLeave={() => setCarouselPaused(false)}>
            {!visiblePipeline
              ? <div className="empty-state">暂无在产项目流水线</div>
              : (
                <div className="project-pipeline-row carousel-card" key={visiblePipeline.project_id}>
                <div className="project-pipeline-head">
                  <div className="project-slot-name">{visiblePipeline.project_name}</div>
                  <span className="sec-kicker">需求 {visiblePipeline.total_issues}</span>
                </div>
                <div className="pipe scroll">
                  {buildPipeline(visiblePipeline).map((p, i) => (
                    <div className="pipe-stage" key={i}>
                      <div className={'pipe-node ' + p.state}><Icon name={p.ic} size={20} /></div>
                      <div className="pipe-cnt" style={{ color: p.state==='active'?'var(--ember)':p.state==='warn'?'var(--amber)':p.state==='done'?'var(--green-soft)':'var(--text-2)' }}>{p.cnt}</div>
                      <div className="pipe-name">{p.name}</div>
                    </div>
                  ))}
                </div>
              </div>
              )}
          </div>
        </div>
      </div>
    </div>
  );
}
