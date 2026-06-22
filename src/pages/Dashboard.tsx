import React, { useState, useEffect, useCallback } from 'react';
import { listen } from '@tauri-apps/api/event';
import Icon from '../components/Icon';
import Select from '../components/Select';
import IntakePanel from '../components/IntakePanel';
import { getPipelineStats, listActiveProjects, listIssues, listTriageIssues, getAutosupplySettings, issueSourceMeta, type PipelineStats, type Project, type Issue, type AutosupplySettings } from '../services';
import { useOperator, DEFAULT_OPERATOR } from '../operator';

// 按本地时段给出问候语前缀。
function greetingPrefix(): string {
  const h = new Date().getHours();
  if (h < 6) return '夜深了';
  if (h < 12) return '上午好';
  if (h < 14) return '中午好';
  if (h < 18) return '下午好';
  return '晚上好';
}

const SEV_COLOR: Record<string, string> = {
  critical: 'red', high: 'amber', medium: 'blue', low: 'green',
  Bug: 'red', Feature: 'ember', Improvement: 'blue', Debt: 'violet',
};

// 需求状态统一中文文案（与 Audit.tsx 的 STATUS_LABEL 对齐，避免中英混用）。
const STATUS_LABEL: Record<string, string> = {
  triage: '待整理',
  pending_analysis: '分析中',
  analysis_failed: '分析失败',
  pending_issue_review: '待需求审核',
  pending_execution: '待执行',
  executing: 'AI 执行中',
  pending_code_review: '待代码审核',
  pending_merge: '待合并',
  execution_failed: '执行失败',
  merge_failed: '合并失败',
  merge_conflict: '合并冲突',
  no_change_needed: '无需改动',
  merged: '已合并',
  reverting: '撤销中',
  reverted: '已撤销',
  rejected: '已拒绝',
};
const STATUS_COLOR: Record<string, string> = {
  triage: '',
  pending_analysis: 'amber',
  analysis_failed: 'red',
  pending_issue_review: 'amber',
  pending_execution: 'amber',
  executing: 'blue',
  pending_code_review: 'ember',
  pending_merge: 'blue',
  execution_failed: 'red',
  merge_failed: 'red',
  merge_conflict: 'amber',
  no_change_needed: 'blue',
  merged: 'green',
  reverting: 'amber',
  reverted: 'violet',
  rejected: 'red',
};

// ── Submit Issue Modal ────────────────────────────────────────────────────────
// Hosts the shared IntakePanel (手动提交 / GitHub / 代码扫描 / 批量导入) so the
// homepage entry stays consistent with the project-level「需求入口」in Audit.
function SubmitIssueModal({ projects, onClose }: { projects: Project[]; onClose: () => void }) {
  const [projectId, setProjectId] = useState(projects[0]?.id ?? '');

  return (
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 60 }}>
      <div style={{ width: 720, maxHeight: 'min(800px, calc(100vh - 32px))', background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', overflow: 'hidden', display: 'flex', flexDirection: 'column' }} onClick={e => e.stopPropagation()}>
        <div style={{ padding: '18px 20px 14px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', gap: 12 }}>
          <div className="eyebrow" style={{ fontSize: 'var(--text-section)', flexShrink: 0 }}><span className="cn">需求入口</span></div>
          <div style={{ marginLeft: 'auto', minWidth: 200 }}>
            <Select className="sm" value={projectId} onChange={setProjectId}
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

// 六个流水线环节的最小计数形状——top-level PipelineStats 与 project_pipelines 元素都满足它。
type StageCounts = {
  triage: number; pending_analysis: number; pending_review_1: number;
  executing: number; pending_review_2: number; merged: number;
};
// 每个环节带 stage（对应需求/CR 状态字段），用于点击节点时按"项目 + 环节"精确跳转到功能审计。
const buildPipeline = (p: StageCounts) => [
  { ic: 'inbox', name: '需求入口',   cnt: p.triage,           stage: 'triage',           state: p.triage > 0 ? 'active' : 'done' },
  { ic: 'search', name: '需求分析',  cnt: p.pending_analysis, stage: 'pending_analysis', state: p.pending_analysis > 0 ? 'active' : 'done' },
  { ic: 'check', name: '需求审核',   cnt: p.pending_review_1, stage: 'pending_issue_review', state: p.pending_review_1 > 0 ? 'active' : 'done' },
  { ic: 'code', name: '代码 Agent', cnt: p.executing,        stage: 'executing',        state: p.executing > 0 ? 'active' : '' },
  { ic: 'eye', name: '代码审核',     cnt: p.pending_review_2, stage: 'pending_code_review', state: p.pending_review_2 > 3 ? 'warn' : p.pending_review_2 > 0 ? 'active' : '' },
  { ic: 'merge', name: '合并 dev',   cnt: p.merged,           stage: 'merged',           state: p.merged > 0 ? 'done' : '' },
];
const PIPE_CNT_COLOR: Record<string, string> = {
  active: 'var(--ember)', warn: 'var(--amber)', done: 'var(--green-soft)', '': 'var(--text-2)',
};

// ── Dashboard ─────────────────────────────────────────────────────────────────
export default function Dashboard({ onOpenInAudit, onOpenStage }: {
  onOpenInAudit: (target: { projectId: string; issueId: string }) => void;
  // 点击完整流水线节点：按"项目 + 环节"跳到功能审计对应视图。
  onOpenStage: (projectId: string, stage: string) => void;
}) {
  const [stats, setStats] = useState<PipelineStats | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [issues, setIssues] = useState<Issue[]>([]);
  const [showSubmit, setShowSubmit] = useState(false);
  const [carouselIndex, setCarouselIndex] = useState(0);
  const [carouselPaused, setCarouselPaused] = useState(false);
  const [triage, setTriage] = useState<Issue[]>([]);
  const [autosupply, setAutosupply] = useState<AutosupplySettings | null>(null);
  const operator = useOperator();

  const loadAll = useCallback(async () => {
    const [s, ps, is, tri, supply] = await Promise.all([
      getPipelineStats(), listActiveProjects(), listIssues(),
      listTriageIssues().catch(() => [] as Issue[]), getAutosupplySettings().catch(() => null),
    ]);
    setTriage(tri);
    setAutosupply(supply);
    setStats(s);
    setProjects(ps);
    setIssues(is.sort((a, b) =>
      new Date(b.created_at).getTime() - new Date(a.created_at).getTime()
    ));
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

  // ── derived ────────────────────────────────────────────────────────────────
  // 队列只看「在途」需求，过滤掉已合并/已拒绝/已撤销等终态（已撤销可在审计页「恢复需求」后重回队列）
  const queueIssues = issues.filter(i => i.status !== 'merged' && i.status !== 'rejected' && i.status !== 'reverted');
  const activeProjectCount = projects.filter(p => p.status === 'active').length;
  const pendingReview = stats?.pending_review_slots ?? stats?.pending_review_2 ?? 0;
  const pauseThreshold = stats?.pause_threshold ?? 20;
  const pressurePct = Math.min(100, (pendingReview / pauseThreshold) * 100);
  const totalSlotCapacity = stats?.total_slot_capacity ?? Math.max(1, activeProjectCount) * (stats?.max_slots ?? 5);
  const projectSlots = stats?.project_slots ?? [];
  const projectPipelines = stats?.project_pipelines ?? [];
  const gatesPending = (stats?.pending_review_1 ?? 0) + (stats?.pending_review_2 ?? 0);
  const carouselCount = Math.max(projectPipelines.length, projectSlots.length);
  const visibleSlot = projectSlots.length ? projectSlots[carouselIndex % projectSlots.length] : null;
  const visiblePipeline = projectPipelines.length ? projectPipelines[carouselIndex % projectPipelines.length] : null;
  const stageLabel = stats?.stage === 'paused' ? '暂停' : stats?.stage === 'throttled' ? '降速' : '正常';
  const stageChip = stats?.stage === 'paused' ? 'red' : stats?.stage === 'throttled' ? 'amber' : 'green';
  const stageBar = stats?.stage === 'paused' ? 'var(--red)' : stats?.stage === 'throttled' ? 'var(--amber)' : 'var(--green)';

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
            <div className="dash-hello">{greetingPrefix()}，{operator.display_name || DEFAULT_OPERATOR.display_name}</div>
            <div className="dash-sub">
              {stats ? `${stats.stage === 'normal' ? '流水线运行正常' : stats.stage === 'throttled' ? '单线程降速中' : '系统已暂停'} · ${stats.active_slots}/${totalSlotCapacity} 项目槽位占用` : '加载中…'}
            </div>
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <button className="btn btn-primary" onClick={() => setShowSubmit(true)}><Icon name="plus" size={16} />提交需求</button>
          </div>
        </div>

        {/* KPI 体征：在产 / 总量 / 待我审核（人类唯一职责，amber 高亮）/ 并发占用 */}
        <div className="stat-grid">
          {[
            { ic: 'box',   color: 'var(--ember)',  val: String(stats?.active_projects ?? activeProjectCount), unit: '', label: '在产项目', delta: `全部 ${projects.length}` },
            { ic: 'inbox', color: 'var(--violet)', val: String(stats?.total_issues ?? issues.length), unit: '', label: '需求总数', delta: '实时数据' },
            { ic: 'clock', color: 'var(--amber)',  val: String(gatesPending), unit: '', label: '待我审核', delta: `需求审核 ${stats?.pending_review_1 ?? 0} · 代码审核 ${stats?.pending_review_2 ?? 0}` },
            { ic: 'cpu',   color: 'var(--blue)',   val: String(stats?.active_slots ?? '—'), unit: `/${totalSlotCapacity}`, label: '并发占用', delta: `每项目 ${stats?.max_slots ?? 5} 槽` },
          ].map((s, i) => (
            <div className="stat" key={i}>
              <div className="stat-ic" style={{ background: `color-mix(in oklab, ${s.color} 16%, transparent)`, color: s.color }}><Icon name={s.ic} size={18} /></div>
              <div className="stat-main">
                <div className="stat-label">{s.label}</div>
                <div className="stat-val">{s.val}<span className="u">{s.unit}</span></div>
              </div>
              <div className="stat-delta up">{s.delta}</div>
            </div>
          ))}
        </div>

        {/* pipeline — 逐项目完整流水线，自动轮播，主页的「看」中枢 */}
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
                    {buildPipeline(visiblePipeline).map((p, i) => {
                      // 计数为 0 的环节没有可看的条目，不可点击跳转。
                      const clickable = p.cnt > 0;
                      return (
                        <div
                          className={'pipe-stage' + (clickable ? ' pipe-clickable' : '')}
                          key={i}
                          onClick={clickable ? () => onOpenStage(visiblePipeline.project_id, p.stage) : undefined}
                          title={clickable ? `查看 ${visiblePipeline.project_name} · ${p.name}（${p.cnt}）` : `${p.name}：暂无`}
                        >
                          <div className={'pipe-node ' + p.state}><Icon name={p.ic} size={20} /></div>
                          <div className="pipe-cnt" style={{ color: PIPE_CNT_COLOR[p.state] }}>{p.cnt}</div>
                          <div className="pipe-name">{p.name}</div>
                        </div>
                      );
                    })}
                  </div>
                </div>
              )}
          </div>
        </div>

        <div className="dash-cols">
          {/* 需求队列 — 最新流入，时间倒序 */}
          <div className="panel">
            <div className="panel-head">
              <div className="panel-title"><Icon name="inbox" size={17} style={{ color: 'var(--ember)' }} />需求队列 · 时间倒序</div>
              <span className="sec-kicker">在途 {queueIssues.length} 条</span>
            </div>
            {queueIssues.length === 0
              ? <div className="empty-compact" style={{ padding: '20px 18px' }}>暂无需求</div>
              : <div className="q-list">{queueIssues.map((q, i) => (
              <div className="q-row" key={q.id} onClick={() => onOpenInAudit({ projectId: q.project_id, issueId: q.id })}
                style={{ cursor: 'pointer' }} title="在功能审计中查看">
                <div className="q-pr">{i + 1}</div>
                <div className="q-main">
                  <div className="q-title">
                    {!!q.restored_from_revert && (
                      <span className="dot" style={{ display: 'inline-block', background: 'var(--violet)', marginRight: 6, verticalAlign: 'middle' }} title="撤销恢复的需求" />
                    )}
                    {q.title}
                  </div>
                  <div className="q-meta">
                    <span className="req-id" style={{ color: 'var(--text-3)' }}>{q.id.slice(0, 10)}</span>
                    <span className={'chip ' + (SEV_COLOR[q.category] || 'blue')} style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }}>{q.category}</span>
                    <span className={'chip ' + (SEV_COLOR[q.severity] || '')} style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }}>{q.severity}</span>
                    {(() => { const s = issueSourceMeta(q.source_type); return (
                      <span className={'chip ' + s.chip} style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }} title={`需求来源：${s.label}`}>{s.label}</span>
                    ); })()}
                    <span>· {projects.find(p => p.id === q.project_id)?.name ?? '—'}</span>
                  </div>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexShrink: 0 }}>
                  <span className={'chip ' + (STATUS_COLOR[q.status] ?? '')}>
                    {STATUS_LABEL[q.status] ?? q.status}
                  </span>
                  <Icon name="chevRight" size={15} style={{ color: 'var(--text-faint)' }} />
                </div>
              </div>
            ))}</div>}
          </div>

          {/* 右列：供料信号（待整理/自动供料，紧邻需求队列）在上，产线运行在下 */}
          <div className="dash-side">
            <div className="ops-sig">
              <div className="ops-chip" role="button" tabIndex={0}
                onClick={() => triage[0] && onOpenInAudit({ projectId: triage[0].project_id, issueId: triage[0].id })}
                title={triage.length ? '在功能审计 → 全量总账中整理' : '待整理池为空'}
                style={{ cursor: triage.length ? 'pointer' : 'default' }}>
                <Icon name="inbox" size={14} style={{ color: triage.length ? 'var(--ember)' : 'var(--text-faint)' }} />
                <span>待整理</span>
                <span className={'chip ' + (triage.length ? 'ember' : '')} style={{ fontSize: 'var(--text-micro)' }}>{triage.length}</span>
              </div>
              <div className="ops-chip">
                <Icon name="refresh" size={14} style={{ color: autosupply?.enabled ? 'var(--green)' : 'var(--text-faint)' }} />
                <span>自动供料</span>
                <span className={'chip ' + (autosupply?.enabled ? 'green' : '')} style={{ fontSize: 'var(--text-micro)' }}>
                  {autosupply?.enabled ? `每 ${autosupply.interval_min}分` : '关'}
                </span>
                {autosupply?.enabled && autosupply.proposer_enabled && <span className="chip blue" style={{ fontSize: 'var(--text-micro)' }}>proposer</span>}
              </div>
            </div>

          {/* 产线运行 — 槽位占用（弹性展示区）+ 背压状态 */}
          <div className="panel ops-panel">
            <div className="panel-head">
              <div className="panel-title"><Icon name="cpu" size={17} style={{ color: 'var(--blue)' }} />产线运行</div>
              <span className="sec-kicker">{stats?.active_slots ?? 0} / {totalSlotCapacity} 占用 · {projectSlots.length ? `${(carouselIndex % projectSlots.length) + 1}/${projectSlots.length}` : '0/0'}</span>
            </div>
            <div className="ops-slots" onMouseEnter={() => setCarouselPaused(true)} onMouseLeave={() => setCarouselPaused(false)}>
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
                        const isPending = occupant?.status === 'pending_code_review';
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
            <div className="ops-foot">
              <div>
                <div className="ops-foot-head">
                  <span className="ops-foot-kicker">背压 · BACKPRESSURE</span>
                  <span className={'chip ' + stageChip}>{stageLabel}</span>
                </div>
                <div className="bp-bar">
                  <div className="bp-seg" style={{ width: `${pressurePct}%`, background: stageBar }} />
                </div>
                <div className="ops-foot-scale">
                  <span>积压 {pendingReview}</span><span>暂停阈值 {pauseThreshold}</span>
                </div>
              </div>
            </div>
          </div>
          </div>
        </div>
      </div>
    </div>
  );
}
