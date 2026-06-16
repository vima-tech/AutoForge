import React, { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { listen } from '@tauri-apps/api/event';
import Icon from '../components/Icon';
import Toast, { type ToastData } from '../components/Toast';
import IntakePanel from '../components/IntakePanel';
import {
  listActiveProjects, listChangeRequests, getWorktreeSession, getCodeDiff, review2, getCrGrade,
  retryChangeRequest, deleteChangeRequest,
  openUrl, listIssues, getIssueAnalysis, review1, parseAnalysisSpec,
  getCrPreview, startCrPreview, stopCrPreview, launchCrApp, getCrPreviewLog,
  listLocalBranches, startBranchPreview, listBranchPreviews, stopBranchPreview, getBranchPreviewLog,
  type Project, type ChangeRequest, type WorktreeSession, type CrGrade,
  type CrPreviewStatus, type Issue, type IssueAnalysis, type IssueAnalysisSpec,
  type BranchInfo, type BranchPreviewStatus,
} from '../services';

type Sel = { kind: 'issue' | 'cr'; id: string };

const STATUS_LABEL: Record<string, string> = {
  pending_review_1: '待需求审核',
  pending_execution: '待执行',
  executing: 'AI 执行中',
  pending_review_2: '待代码审核',
  pending_merge: '待合并',
  execution_failed: '执行失败',
  merge_failed: '合并失败',
  merged: '已合并',
  rejected: '已拒绝',
};
const STATUS_COLOR: Record<string, string> = {
  pending_review_1: 'amber',
  pending_execution: 'amber',
  executing: 'blue',
  pending_review_2: 'ember',
  pending_merge: 'blue',
  execution_failed: 'red',
  merge_failed: 'red',
  merged: 'green',
  rejected: 'red',
};
// Failed states float to the top so abnormal requirements are easy to find and resolve.
const STATUS_ORDER = ['execution_failed', 'merge_failed', 'pending_review_2', 'executing', 'pending_execution', 'pending_merge', 'pending_review_1', 'merged', 'rejected'];

// Stuck/abnormal CR states that the user can recover (retry) or remove (delete).
const FAILED_STATUSES = ['execution_failed', 'merge_failed'];

const SEV_COLOR: Record<string, string> = {
  critical: 'red', high: 'amber', medium: 'blue', low: 'green',
  Bug: 'red', Feature: 'ember', Improvement: 'blue', Debt: 'violet',
};

function sortedCrs(crs: ChangeRequest[]) {
  return [...crs].sort((a, b) => {
    const ai = STATUS_ORDER.indexOf(a.status);
    const bi = STATUS_ORDER.indexOf(b.status);
    if (ai !== bi) return (ai === -1 ? 99 : ai) - (bi === -1 ? 99 : bi);
    return b.updated_at.localeCompare(a.updated_at);
  });
}

function parseReport(md: string) {
  const summary = (md.match(/##\s*改动摘要\n([\s\S]*?)(?=##|$)/) ?? [])[1]?.trim() ?? '';
  const filesSection = (md.match(/##\s*修改文件列表\n([\s\S]*?)(?=##|$)/) ?? [])[1]?.trim() ?? '';
  const testsSection = (md.match(/##\s*测试情况\n([\s\S]*?)(?=##|$)/) ?? [])[1]?.trim() ?? '';
  const risk = (md.match(/##\s*潜在风险\n([\s\S]*?)(?=##|$)/) ?? [])[1]?.trim() ?? '';
  const files = filesSection.split('\n').filter(l => l.startsWith('-')).map(l => {
    const m = l.match(/^-\s*(.+?):\s*.+?\(?\+(\d+)(?:\s*-(\d+))?\)?/);
    return m ? { name: m[1], add: parseInt(m[2]), del: parseInt(m[3] ?? '0') } : null;
  }).filter(Boolean) as { name: string; add: number; del: number }[];
  return { summary, files, testsSection, risk };
}

interface DiffLine { n1: number|''; n2: number|''; t: 'add'|'del'|'ctx'; code: string }
interface Hunk { file: string; hunk: string; lines: DiffLine[] }
function parseDiff(raw: string): Hunk[] {
  const hunks: Hunk[] = [];
  let curFile = '';
  let curHunk: Hunk | null = null;
  let n1 = 0, n2 = 0;
  for (const line of raw.split('\n')) {
    if (line.startsWith('diff --git ')) { curFile = ''; continue; }
    if (line.startsWith('--- ')) { curFile = line.slice(4).replace(/^a\//, ''); continue; }
    if (line.startsWith('+++ ')) continue;
    if (line.startsWith('@@ ')) {
      const m = line.match(/@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
      n1 = m ? parseInt(m[1]) : 0;
      n2 = m ? parseInt(m[2]) : 0;
      curHunk = { file: curFile, hunk: line, lines: [] };
      hunks.push(curHunk);
      continue;
    }
    if (!curHunk) continue;
    if (line.startsWith('+')) curHunk.lines.push({ n1: '', n2: n2++, t: 'add', code: line.slice(1) });
    else if (line.startsWith('-')) curHunk.lines.push({ n1: n1++, n2: '', t: 'del', code: line.slice(1) });
    else curHunk.lines.push({ n1: n1++, n2: n2++, t: 'ctx', code: line.slice(1) });
  }
  return hunks;
}

// ── ResizeHandle ─────────────────────────────────────────────────────────────

function ResizeHandle({ onDrag }: { onDrag: (dx: number) => void }) {
  const [active, setActive] = useState(false);

  const onMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    let last = e.clientX;
    setActive(true);
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';

    const onMove = (e: MouseEvent) => {
      onDrag(e.clientX - last);
      last = e.clientX;
    };
    const onUp = () => {
      setActive(false);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
    };
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  };

  return <div className={`resize-handle${active ? ' active' : ''}`} onMouseDown={onMouseDown} />;
}

// ── LiveLogModal ───────────────────────────────────────────────────────────────

// 去除 ANSI/OSC 转义序列（cargo/tauri 输出含大量颜色码），并折叠进度条的 \r 覆盖。
function stripAnsi(s: string): string {
  return s
    // eslint-disable-next-line no-control-regex
    .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, '')
    // eslint-disable-next-line no-control-regex
    .replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, '');
}
function logLineTone(line: string): string {
  const t = line.trimStart();
  // 日志头部：命令、cwd/PATH 注释、分隔线 —— 作次要信息淡化
  if (t.startsWith('$ ') || t.startsWith('# ') || /^[-=]{6,}\s*$/.test(t)) return 'cmd';
  const l = line.toLowerCase();
  if (/(error|panic|fatal|failed|✗|\bcannot\b|exception|traceback|\berr\b)/.test(l)) return 'err';
  if (/(warn|warning|deprecated)/.test(l)) return 'warn';
  if (/(compiling|building|finished|running|ready|✓|local:|listening|started|success|\bdone\b)/.test(l)) return 'ok';
  return '';
}
function parseLogLines(raw: string): { text: string; tone: string }[] {
  const arr = stripAnsi(raw).split('\n').map(seg => {
    const text = seg.includes('\r') ? seg.slice(seg.lastIndexOf('\r') + 1) : seg;
    return { text, tone: logLineTone(text) };
  });
  // 去掉末尾多余空行，避免一长串空白尾巴
  while (arr.length > 1 && arr[arr.length - 1].text === '') arr.pop();
  return arr;
}

// 实时日志窗口：定时拉取、按行高亮、跟随底部自动滚动（用户上滚则暂停跟随）。
function LiveLogModal({ title, load, onClose }: {
  title: string; load: () => Promise<string>; onClose: () => void;
}) {
  const [raw, setRaw] = useState('');
  const bodyRef = useRef<HTMLDivElement>(null);
  const followRef = useRef(true);
  const loadRef = useRef(load);
  loadRef.current = load;
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  // 仅挂载时启动轮询（组件按 sig key 重挂载切换目标），避免父组件 re-render 重置定时器
  useEffect(() => {
    let alive = true;
    const tick = () => { loadRef.current().then(t => { if (alive) setRaw(t); }).catch(() => {}); };
    tick();
    const id = window.setInterval(tick, 1200);
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') closeRef.current(); };
    window.addEventListener('keydown', onKey);
    return () => { alive = false; window.clearInterval(id); window.removeEventListener('keydown', onKey); };
  }, []);

  const lines = useMemo(() => parseLogLines(raw), [raw]);

  // 跟随底部：内容更新后若仍处于跟随态则滚到底
  useEffect(() => {
    const el = bodyRef.current;
    if (el && followRef.current) el.scrollTop = el.scrollHeight;
  }, [lines]);

  const onScroll = () => {
    const el = bodyRef.current;
    if (el) followRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
  };

  return (
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 240 }}>
      <div style={{ width: 820, maxWidth: 'calc(100vw - 32px)', height: 'min(78vh, 640px)', background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 16, boxShadow: 'var(--shadow-lg)', overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
        <div style={{ padding: '12px 16px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', gap: 8 }}>
          <span className="dot green" style={{ flexShrink: 0 }} />
          <span className="eyebrow" style={{ fontSize: 'var(--text-section)' }}><span className="cn">{title}</span></span>
          <span style={{ marginLeft: 'auto', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-faint)' }}>
            {raw ? `${lines.length} 行 · ` : ''}实时跟随
          </span>
          <button className="icon-btn" onClick={onClose}><Icon name="x" size={18} /></button>
        </div>
        <div ref={bodyRef} onScroll={onScroll} className="log-body scroll">
          {lines.length <= 1 && !raw
            ? <div className="log-empty">（暂无日志输出 —— 进程可能尚未启动，或启动命令本身未产生输出）</div>
            : lines.map((l, i) => (
                <div key={i} className={'log-line' + (l.tone ? ' ' + l.tone : '')}>
                  <span className="log-gut">{i + 1}</span>
                  <span className="log-code">{l.text || ' '}</span>
                </div>
              ))}
        </div>
      </div>
    </div>
  );
}

// ── BranchLauncher（页头：启动项目 + 运行中分支）────────────────────────────────

function BranchLauncher({ branches, branchPreviews, onStart, onStop, onShowLog }: {
  branches: BranchInfo[]; branchPreviews: BranchPreviewStatus[];
  onStart: (b: string) => void; onStop: (b: string) => void; onShowLog: (b: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: PointerEvent) => {
      if (e.target instanceof Node && ref.current?.contains(e.target)) return;
      setOpen(false);
    };
    document.addEventListener('pointerdown', close);
    return () => document.removeEventListener('pointerdown', close);
  }, [open]);

  return (
    <div className="audit-launch" ref={ref}>
      {branchPreviews.length > 0 && (
        <div className="audit-launch-runs">
          {branchPreviews.map(p => (
            <div key={p.branch} className="run-pill">
              <span className={'dot ' + (p.status === 'running' ? 'green' : 'amber')} style={{ flexShrink: 0 }} />
              <span className="run-pill-name" title={p.branch}>{p.branch}</span>
              {p.kind === 'tauri'
                ? <span className="chip ember" style={{ flexShrink: 0, fontSize: 'var(--text-micro)', padding: '1px 6px' }}>APP</span>
                : p.url && (
                  <button className="icon-btn run-pill-btn" title="在浏览器打开" onClick={() => openUrl(p.url!).catch(() => {})}>
                    <Icon name="external" size={13} />
                  </button>
                )}
              <button className="icon-btn run-pill-btn" title="查看启动日志" onClick={() => onShowLog(p.branch)}>
                <Icon name="log" size={13} />
              </button>
              <button className="icon-btn run-pill-btn" title="停止" onClick={() => onStop(p.branch)}>
                <Icon name="x" size={13} />
              </button>
            </div>
          ))}
        </div>
      )}
      <div style={{ position: 'relative', flexShrink: 0 }}>
        <button className="btn btn-sm" onClick={() => setOpen(o => !o)} title="选择分支启动（main 即线上版本）">
          <Icon name="play" size={14} />启动项目
          <Icon name="chevDown" size={13} style={{ color: 'var(--text-3)', transition: 'transform .15s', transform: open ? 'rotate(180deg)' : 'none' }} />
        </button>
        {open && (
          <div className="mention-pop" style={{ right: 0, left: 'auto', top: 'calc(100% + 6px)', bottom: 'auto', minWidth: 220, marginBottom: 0 }}>
            {branches.length === 0 && (
              <div className="empty-compact" style={{ padding: '8px 10px' }}>无本地分支或未配置启动命令</div>
            )}
            {branches.map(b => {
              const running = branchPreviews.some(p => p.branch === b.name);
              return (
                <div key={b.name} className="mention-row" onClick={() => { onStart(b.name); setOpen(false); }}>
                  <Icon name="merge" size={14} style={{ color: 'var(--text-3)', flexShrink: 0 }} />
                  <div style={{ minWidth: 0, flex: 1 }}>
                    <div className="nm" style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
                      <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{b.name}</span>
                      {b.is_main && <span className="chip ember" style={{ padding: '0 5px', fontSize: 'var(--text-micro)', flexShrink: 0 }}>线上</span>}
                      {b.is_dev && <span className="chip blue" style={{ padding: '0 5px', fontSize: 'var(--text-micro)', flexShrink: 0 }}>dev</span>}
                    </div>
                  </div>
                  {running && <span className="dot green" style={{ flexShrink: 0 }} />}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

// ── AuditList ────────────────────────────────────────────────────────────────

function AuditList({ projects, activeProject, setActiveProject, projectReviewCounts, crs, pendingIssues, issueTitles, sel,
  onSelectCr, onSelectIssue, onOpenIntake,
  width }: {
  projects: Project[]; activeProject: Project | null; setActiveProject: (p: Project) => void;
  projectReviewCounts: Record<string, number>; crs: ChangeRequest[]; pendingIssues: Issue[];
  issueTitles: Record<string, string>; sel: Sel | null;
  onSelectCr: (id: string) => void; onSelectIssue: (id: string) => void;
  onOpenIntake: () => void;
  width: number;
}) {
  const [open, setOpen] = useState(false);
  const menuRef = React.useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: PointerEvent) => {
      if (!(e.target instanceof Node)) return;
      if (menuRef.current?.contains(e.target)) return;
      setOpen(false);
    };
    document.addEventListener('pointerdown', close);
    return () => document.removeEventListener('pointerdown', close);
  }, [open]);

  return (
    <div className="list-col" style={{ width, flex: `0 0 ${width}px` }}>
      <div className="audit-proj" ref={menuRef}>
        <div style={{ position: 'relative' }}>
          {activeProject ? (
            <div className="proj-select" onClick={() => setOpen(o => !o)} style={{ cursor: 'pointer' }}>
              <div className="proj-logo" style={{ background: '#e8772e' }}>{activeProject.name[0]}</div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div className="proj-name">{activeProject.name}</div>
                <div className="proj-meta">{activeProject.description}</div>
              </div>
              <Icon name="chevDown" size={16} style={{ color: 'var(--text-3)', transition: 'transform .15s', transform: open ? 'rotate(180deg)' : 'none' }} />
            </div>
          ) : (
            <div className="empty-compact" style={{ padding: '8px 10px' }}>
              暂无项目，请前往「项目管理」页添加
            </div>
          )}
          {open && (
          <div className="mention-pop audit-project-pop" style={{ left: 0, right: 0, top: 'calc(100% + 6px)', bottom: 'auto', width: '100%', marginBottom: 0 }}>
            {projects.map(p => (
              <div key={p.id} className="mention-row" onClick={() => { setActiveProject(p); setOpen(false); }}>
                <div className="proj-logo" style={{ background: '#e8772e', width: 28, height: 28, fontSize: 'var(--text-label)', borderRadius: 8 }}>{p.name[0]}</div>
                <div style={{ minWidth: 0, flex: 1 }}>
                  <div className="nm" style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
                    {p.name}
                    {p.id === activeProject?.id && (
                      <span style={{ width: 5, height: 5, borderRadius: '50%', background: 'var(--ember)', display: 'inline-block', flexShrink: 0 }} />
                    )}
                  </div>
                  <div className="rl">{p.description || p.slug}</div>
                </div>
                {(projectReviewCounts[p.id] ?? 0) > 0 && (
                  <span className="chip amber" style={{ padding: '1px 6px', fontSize: 'var(--text-micro)', flexShrink: 0 }}>
                    {projectReviewCounts[p.id]}
                  </span>
                )}
              </div>
            ))}
          </div>
          )}
        </div>
      </div>

      <div className="list-group-label" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8 }}>
        <span>全部需求 · {crs.length + pendingIssues.length} 条</span>
        <button className="btn btn-sm" style={{ padding: '3px 9px' }} disabled={!activeProject} onClick={onOpenIntake} title="提交 / 同步需求到当前项目">
          <Icon name="inbox" size={13} />需求入口
        </button>
      </div>
      <div className="list-body scroll" style={{ paddingTop: 0 }}>
        {crs.length === 0 && pendingIssues.length === 0 && <div className="empty-compact">暂无需求</div>}

        {/* 审核 1：待需求审核（Issue，尚未生成 CR） */}
        {pendingIssues.length > 0 && (
          <div style={{ padding: '8px 12px 4px', fontSize: 'var(--text-caption)', letterSpacing: '.06em', textTransform: 'uppercase', color: 'var(--text-faint)', fontWeight: 600 }}>
            {STATUS_LABEL.pending_review_1}
          </div>
        )}
        {pendingIssues.map(issue => (
          <div key={issue.id} className={'req-item' + (sel?.kind === 'issue' && sel.id === issue.id ? ' active' : '')} onClick={() => onSelectIssue(issue.id)}>
            <div className="req-item-top">
              <span className="req-id">{issue.id.slice(0, 8)}</span>
              <span className="chip amber" style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }}>审核 1</span>
              <span className="req-time">{new Date(issue.updated_at).toLocaleString('zh', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</span>
            </div>
            <div className="req-title" style={{ fontSize: 'var(--text-control)' }} title={issue.title}>{issue.title}</div>
          </div>
        ))}

        {/* 审核 2 及其它 CR 状态 */}
        {(() => {
          const sorted = sortedCrs(crs);
          let lastStatus = '';
          return sorted.map(r => {
            const showLabel = r.status !== lastStatus;
            lastStatus = r.status;
            return (
              <React.Fragment key={r.id}>
                {showLabel && (
                  <div style={{ padding: '8px 12px 4px', fontSize: 'var(--text-caption)', letterSpacing: '.06em', textTransform: 'uppercase', color: 'var(--text-faint)', fontWeight: 600 }}>
                    {STATUS_LABEL[r.status] ?? r.status}
                  </div>
                )}
                <div className={'req-item' + (sel?.kind === 'cr' && sel.id === r.id ? ' active' : '')} onClick={() => onSelectCr(r.id)}>
                  <div className="req-item-top">
                    <span className="req-id">{r.id.slice(0, 8)}</span>
                    <span className={'chip ' + (STATUS_COLOR[r.status] ?? '')} style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }}>{STATUS_LABEL[r.status] ?? r.status}</span>
                    <span className="req-time">{new Date(r.updated_at).toLocaleString('zh', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</span>
                  </div>
                  <div className="req-title" style={{ fontSize: 'var(--text-control)' }} title={issueTitles[r.issue_id] || r.issue_id.slice(0, 8)}>{issueTitles[r.issue_id] || r.issue_id.slice(0, 8)}</div>
                </div>
              </React.Fragment>
            );
          });
        })()}
      </div>
    </div>
  );
}

// ── IssueReviewView (审核 1：需求审核) ─────────────────────────────────────────

function ScorePill({ label, value }: { label: string; value: number | null }) {
  if (value === null || value === undefined) return null;
  const pct = value <= 1 ? Math.round(value * 100) : Math.round(value);
  const color = pct >= 75 ? 'var(--green)' : pct >= 50 ? 'var(--amber)' : 'var(--red)';
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4, background: 'var(--bg-3)', borderRadius: 10, padding: '10px 14px', minWidth: 100 }}>
      <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', letterSpacing: '.06em', textTransform: 'uppercase' }}>{label}</span>
      <span style={{ fontSize: 'var(--text-section)', fontWeight: 700, color, fontFamily: 'var(--font-display)' }}>{pct}<span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)' }}>%</span></span>
    </div>
  );
}

const CHANGE_CHIP: Record<string, string> = { add: 'green', modify: 'blue', delete: 'red', investigate: 'violet' };
const RISK_CHIP: Record<string, string> = { high: 'red', medium: 'amber', low: 'blue' };

function SpecH2({ icon, color, children }: { icon: string; color: string; children: React.ReactNode }) {
  return <h2><Icon name={icon} size={18} style={{ color }} />{children}</h2>;
}

const liStyle: React.CSSProperties = { fontSize: 'var(--text-control)', color: 'var(--text-2)', lineHeight: 'var(--leading-normal)' };
const monoPath: React.CSSProperties = { fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text)' };

function AnalysisSpecView({ spec }: { spec: IssueAnalysisSpec }) {
  const [briefOpen, setBriefOpen] = useState(false);
  const u = spec.understanding, rc = spec.root_cause, sc = spec.scope;
  const plan = spec.implementation_plan, b = spec.claude_code_brief;
  const steps = [...plan.steps].sort((a, z) => a.order - z.order);

  return (
    <>
      {(u.restated_requirement || u.reproduction_steps.length > 0) && (
        <>
          <SpecH2 icon="search" color="var(--blue)">需求理解</SpecH2>
          {u.problem_type && <p style={{ margin: '0 0 8px' }}><span className="chip">{u.problem_type}</span></p>}
          {u.restated_requirement && <p style={{ whiteSpace: 'pre-line' }}>{u.restated_requirement}</p>}
          {u.current_behavior && <p style={liStyle}><b>当前行为：</b>{u.current_behavior}</p>}
          {u.expected_behavior && <p style={liStyle}><b>期望行为：</b>{u.expected_behavior}</p>}
          {u.reproduction_steps.length > 0 && (
            <ol style={{ paddingLeft: 18, margin: '6px 0', display: 'flex', flexDirection: 'column', gap: 3 }}>
              {u.reproduction_steps.map((s, i) => <li key={i} style={liStyle}>{s}</li>)}
            </ol>
          )}
        </>
      )}

      {rc && rc.hypothesis && (
        <>
          <SpecH2 icon="alert" color="var(--amber)">根因分析</SpecH2>
          <p style={{ whiteSpace: 'pre-line' }}>{rc.hypothesis}</p>
          {rc.suspected_locations.length > 0 && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6, margin: '6px 0' }}>
              {rc.suspected_locations.map((l, i) => (
                <div key={i} style={liStyle}>
                  <span style={monoPath}>{l.file}{l.symbol ? ` :: ${l.symbol}` : ''}</span>
                  <span style={{ color: 'var(--text-3)' }}> — {l.reason}</span>
                </div>
              ))}
            </div>
          )}
        </>
      )}

      {(sc.affected_files.length > 0 || sc.related_files.length > 0) && (
        <>
          <SpecH2 icon="file" color="var(--violet)">影响文件{sc.blast_radius ? <span className="chip" style={{ marginLeft: 8, fontSize: 'var(--text-micro)' }}>{sc.blast_radius}</span> : null}</SpecH2>
          {sc.affected_files.length > 0 && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              {sc.affected_files.map((f, i) => (
                <div key={i} style={{ display: 'flex', alignItems: 'baseline', gap: 8, flexWrap: 'wrap' }}>
                  <span className={'chip ' + (CHANGE_CHIP[f.change_type] || '')} style={{ fontSize: 'var(--text-micro)' }}>{f.change_type}</span>
                  <span style={monoPath}>{f.path}</span>
                  <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>{f.reason}</span>
                </div>
              ))}
            </div>
          )}
          {sc.related_files.length > 0 && (
            <div style={{ marginTop: sc.affected_files.length > 0 ? 10 : 0 }}>
              <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', letterSpacing: '.06em', textTransform: 'uppercase', marginBottom: 4 }}>相关文件（需阅读，不一定改动）</div>
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
                {sc.related_files.map((p, i) => (
                  <span key={i} className="chip" style={{ ...monoPath, fontSize: 'var(--text-micro)' }}>{p}</span>
                ))}
              </div>
            </div>
          )}
          {sc.entry_points.length > 0 && <p style={{ ...liStyle, marginTop: 8 }}><b>入手点：</b>{sc.entry_points.join('；')}</p>}
          {sc.out_of_scope.length > 0 && <p style={liStyle}><b>不在范围：</b>{sc.out_of_scope.join('；')}</p>}
        </>
      )}

      {(plan.approach || steps.length > 0) && (
        <>
          <SpecH2 icon="layers" color="var(--ember)">实现计划</SpecH2>
          {plan.approach && <p style={{ whiteSpace: 'pre-line' }}>{plan.approach}</p>}
          {steps.length > 0 && (
            <ol style={{ paddingLeft: 18, margin: '6px 0', display: 'flex', flexDirection: 'column', gap: 5 }}>
              {steps.map((s, i) => (
                <li key={i} style={liStyle}>
                  {s.action}
                  {s.target_files.length > 0 && <span style={{ ...monoPath, color: 'var(--text-3)' }}> （{s.target_files.join(', ')}）</span>}
                  {s.details && <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>{s.details}</div>}
                </li>
              ))}
            </ol>
          )}
          {plan.data_model_changes.filter(d => d.kind !== 'none' && d.description).map((d, i) => (
            <p key={i} style={liStyle}><span className="chip violet" style={{ fontSize: 'var(--text-micro)' }}>{d.kind}</span> {d.description}</p>
          ))}
          {plan.new_dependencies.length > 0 && <p style={liStyle}><b style={{ color: 'var(--amber)' }}>新增依赖：</b>{plan.new_dependencies.join(', ')}</p>}
        </>
      )}

      {spec.acceptance_criteria.length > 0 && (
        <>
          <SpecH2 icon="check" color="var(--green)">验收标准</SpecH2>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
            {spec.acceptance_criteria.map((ac, i) => (
              <div key={i} style={liStyle}><span style={{ fontFamily: 'var(--font-mono)', color: 'var(--green)', fontSize: 'var(--text-caption)' }}>{ac.id}</span> {ac.statement}</div>
            ))}
          </div>
        </>
      )}

      {(spec.constraints.must.length > 0 || spec.constraints.must_not.length > 0) && (
        <>
          <SpecH2 icon="shield" color="var(--blue)">约束</SpecH2>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            {spec.constraints.must.map((m, i) => <div key={'m' + i} style={liStyle}><span style={{ color: 'var(--green)' }}>✓</span> {m}</div>)}
            {spec.constraints.must_not.map((m, i) => <div key={'n' + i} style={liStyle}><span style={{ color: 'var(--red)' }}>✕</span> {m}</div>)}
          </div>
        </>
      )}

      {spec.risks.length > 0 && (
        <>
          <SpecH2 icon="alert" color="var(--red)">风险</SpecH2>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
            {spec.risks.map((r, i) => (
              <div key={i} style={liStyle}>
                <span className={'chip ' + (RISK_CHIP[r.severity] || '')} style={{ fontSize: 'var(--text-micro)' }}>{r.severity}</span> {r.description}
                {r.mitigation && <span style={{ color: 'var(--text-3)' }}>（缓解：{r.mitigation}）</span>}
              </div>
            ))}
          </div>
        </>
      )}

      {spec.open_questions.length > 0 && (
        <div className="iter-warn" style={{ marginTop: 14 }}>
          <Icon name="alert" size={20} />
          <div>
            <b>待澄清（批准前请确认）</b>
            <ul style={{ paddingLeft: 18, margin: '4px 0 0', display: 'flex', flexDirection: 'column', gap: 2 }}>
              {spec.open_questions.map((q, i) => <li key={i}>{q}</li>)}
            </ul>
          </div>
        </div>
      )}

      {(b.objective || b.instructions.length > 0) && (
        <div style={{ marginTop: 14 }}>
          <button className="btn btn-sm" onClick={() => setBriefOpen(o => !o)}>
            <Icon name={briefOpen ? 'eye-off' : 'eye'} size={13} />{briefOpen ? '收起' : '查看'} Claude Code 执行工单
          </button>
          {briefOpen && (
            <div className="panel" style={{ marginTop: 8, padding: '12px 14px' }}>
              {b.objective && <p style={{ margin: '0 0 8px' }}><b>目标：</b>{b.objective}</p>}
              {b.instructions.length > 0 && (
                <ol style={{ paddingLeft: 18, margin: '0 0 8px', display: 'flex', flexDirection: 'column', gap: 3 }}>
                  {b.instructions.map((s, i) => <li key={i} style={liStyle}>{s}</li>)}
                </ol>
              )}
              {b.do.map((d, i) => <div key={'d' + i} style={liStyle}><span style={{ color: 'var(--green)' }}>✓</span> {d}</div>)}
              {b.dont.map((d, i) => <div key={'x' + i} style={liStyle}><span style={{ color: 'var(--red)' }}>✕</span> {d}</div>)}
              {b.definition_of_done.length > 0 && (
                <p style={{ ...liStyle, marginTop: 8 }}><b>完成判定：</b>{b.definition_of_done.join('；')}</p>
              )}
            </div>
          )}
        </div>
      )}
    </>
  );
}

function IssueReviewView({ issue, analysis, analysisLoading, submitting, decided, advice, setAdvice, onDecide }: {
  issue: Issue; analysis: IssueAnalysis | null; analysisLoading: boolean;
  submitting: boolean; decided: string | null;
  advice: string; setAdvice: (v: string) => void;
  onDecide: (decision: 'approved' | 'rejected') => void;
}) {
  const canReview = issue.status === 'pending_review_1' && !decided;
  const spec = parseAnalysisSpec(analysis?.analysis_json);
  return (
    <>
      <div className="audit-top">
        <div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span className="req-id" style={{ fontSize: 'var(--text-control)' }}>{issue.id.slice(0, 10)}</span>
            <span style={{ fontWeight: 700, fontSize: 'var(--text-title)' }}>{issue.title}</span>
            <span className="chip amber">审核 1 · 需求审核</span>
          </div>
          <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 2, display: 'flex', gap: 8 }}>
            <span className={'chip ' + (SEV_COLOR[issue.category] || 'blue')} style={{ padding: '0 7px', fontSize: 'var(--text-micro)' }}>{issue.category}</span>
            <span className={'chip ' + (SEV_COLOR[issue.severity] || '')} style={{ padding: '0 7px', fontSize: 'var(--text-micro)' }}>{issue.severity}</span>
            <span>{new Date(issue.updated_at).toLocaleString('zh')}</span>
          </div>
        </div>
        <div className="audit-decide">
          {decided
            ? <span className={'chip ' + (decided === 'approved' ? 'green' : 'red')} style={{ padding: '7px 14px', fontSize: 'var(--text-control)' }}>
                <Icon name={decided === 'approved' ? 'check' : 'x'} size={14} />
                {decided === 'approved' ? '已批准 · 进入编码' : '已拒绝'}
              </span>
            : canReview
              ? <>
                  <button className="btn btn-danger" onClick={() => onDecide('rejected')} disabled={submitting}><Icon name="x" size={15} />拒绝</button>
                  <button className="btn btn-primary" onClick={() => onDecide('approved')} disabled={submitting}><Icon name="check" size={15} />批准 · 进入编码</button>
                </>
              : <span className="chip" style={{ padding: '7px 14px', fontSize: 'var(--text-control)' }}>{STATUS_LABEL[issue.status] ?? issue.status}</span>}
        </div>
      </div>

      <div className="diff-viewport scroll" style={{ flex: 1 }}>
        <div className="report" style={{ maxWidth: 760 }}>
          <h2><Icon name="inbox" size={18} style={{ color: 'var(--ember)' }} />需求描述</h2>
          <p style={{ whiteSpace: 'pre-line' }}>{issue.description || '（无描述）'}</p>

          {analysisLoading ? (
            <div className="empty-compact" style={{ padding: '20px 0' }}>加载分析…</div>
          ) : analysis ? (
            <>
              <h2><Icon name="search" size={18} style={{ color: 'var(--blue)' }} />分析摘要</h2>
              <p style={{ whiteSpace: 'pre-line' }}>{analysis.analysis_summary || '（无摘要）'}</p>

              <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap', margin: '12px 0' }}>
                <ScorePill label="真实性" value={analysis.authenticity_score} />
                <ScorePill label="可行性" value={analysis.feasibility_score} />
                {analysis.priority_suggestion !== null && (
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 4, background: 'var(--bg-3)', borderRadius: 10, padding: '10px 14px', minWidth: 100 }}>
                    <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', letterSpacing: '.06em', textTransform: 'uppercase' }}>建议优先级</span>
                    <span style={{ fontSize: 'var(--text-section)', fontWeight: 700, color: 'var(--ember)', fontFamily: 'var(--font-display)' }}>{analysis.priority_suggestion}</span>
                  </div>
                )}
              </div>

              {(analysis.category_suggestion || analysis.severity_suggestion) && (
                <p style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <span style={{ color: 'var(--text-3)' }}>AI 建议：</span>
                  {analysis.category_suggestion && <span className={'chip ' + (SEV_COLOR[analysis.category_suggestion] || 'blue')}>{analysis.category_suggestion}</span>}
                  {analysis.severity_suggestion && <span className={'chip ' + (SEV_COLOR[analysis.severity_suggestion] || '')}>{analysis.severity_suggestion}</span>}
                </p>
              )}

              {spec
                ? <AnalysisSpecView spec={spec} />
                : analysis.affected_modules && (
                    <><h2><Icon name="layers" size={18} style={{ color: 'var(--violet)' }} />影响模块</h2><p style={{ whiteSpace: 'pre-line' }}>{analysis.affected_modules}</p></>
                  )}

              {analysis.duplicate_of && (
                <div className="iter-warn"><Icon name="alert" size={20} /><div>疑似重复需求：<b>{analysis.duplicate_of.slice(0, 10)}</b>。批准前请确认是否与既有需求重叠。</div></div>
              )}
            </>
          ) : (
            <div className="empty-compact" style={{ padding: '20px 0' }}>暂无分析结果</div>
          )}

          <div style={{ marginTop: 18 }}>
            <div className="advice-label" style={{ marginBottom: 6 }}>管理员建议 → 编码 Agent（可选）</div>
            <textarea
              value={advice}
              onChange={e => setAdvice(e.target.value)}
              placeholder={canReview ? '批准时附带的实现指引、约束或注意事项…' : '只读状态'}
              disabled={!canReview}
              style={{ width: '100%', boxSizing: 'border-box', minHeight: 80, background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 9, padding: '10px 12px', color: 'var(--text)', fontFamily: 'var(--font-sans)', fontSize: 'var(--text-control)', resize: 'vertical', outline: 'none' }}
            />
          </div>
        </div>
      </div>
    </>
  );
}

// ── AuditPage ────────────────────────────────────────────────────────────────

export default function AuditPage({ target, onTargetConsumed }: {
  target: { projectId: string; issueId: string } | null;
  onTargetConsumed: () => void;
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [activeProject, setActiveProject] = useState<Project | null>(null);
  const [crs, setCrs] = useState<ChangeRequest[]>([]);
  const [pendingIssues, setPendingIssues] = useState<Issue[]>([]);
  const [issueTitles, setIssueTitles] = useState<Record<string, string>>({});
  const [issuesById, setIssuesById] = useState<Record<string, Issue>>({});
  const [origReqOpen, setOrigReqOpen] = useState(false);
  const [sel, setSel] = useState<Sel | null>(null);
  const [loadedProjectId, setLoadedProjectId] = useState('');
  const [issueAnalysis, setIssueAnalysis] = useState<IssueAnalysis | null>(null);
  const [analysisLoading, setAnalysisLoading] = useState(false);
  const [session, setSession] = useState<WorktreeSession | null>(null);
  const [crPreview, setCrPreview] = useState<CrPreviewStatus | null>(null);
  const [branches, setBranches] = useState<BranchInfo[]>([]);
  const [branchPreviews, setBranchPreviews] = useState<BranchPreviewStatus[]>([]);
  const [diff, setDiff] = useState('');
  const [grade, setGrade] = useState<CrGrade | null>(null);
  const [diffMode, setDiffMode] = useState<'unified' | 'split'>('unified');
  const [tab, setTab] = useState<'report' | 'diff'>('report');
  const [advice, setAdvice] = useState('');
  const [decided, setDecided] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [crLoading, setCrLoading] = useState(false);
  const [projectReviewCounts, setProjectReviewCounts] = useState<Record<string, number>>({});
  const [intakeOpen, setIntakeOpen] = useState(false);
  const [logModal, setLogModal] = useState<{ title: string; sig: string; load: () => Promise<string> } | null>(null);
  const [toast, setToast] = useState<ToastData | null>(null);
  // 统一系统内提示框：替代浏览器原生 alert()
  const showError = useCallback((msg: string) => setToast({ msg, tone: 'error' }), []);

  // Column widths（左侧列表；右侧 audit-right 已移除）
  const [listWidth, setListWidth] = useState(300);

  // Advice textarea ref for auto-focus
  const adviceRef = useRef<HTMLTextAreaElement>(null);
  // Monotonic token to discard stale CR-detail responses (handles same-id refetches after a revision)
  const loadReqRef = useRef(0);
  // web 预览：startCrPreview 后服务还在 starting，置位以便就绪时自动打开浏览器
  const autoOpenRef = useRef(false);

  const activeCr = sel?.kind === 'cr' ? sel.id : '';
  const activeIssueId = sel?.kind === 'issue' ? sel.id : '';
  // 选中 CR 的 updated_at：修改/重新执行后会变化，用作 diff 重新拉取的信号
  const activeCrUpdatedAt = sel?.kind === 'cr' ? crs.find(c => c.id === sel.id)?.updated_at : undefined;

  const loadProjectReviewCounts = useCallback(async () => {
    const pending = await listChangeRequests(undefined, 'pending_review_2');
    setProjectReviewCounts(pending.reduce<Record<string, number>>((acc, cr) => {
      acc[cr.project_id] = (acc[cr.project_id] ?? 0) + 1;
      return acc;
    }, {}));
  }, []);

  const loadProjects = useCallback(async () => {
    const ps = await listActiveProjects();
    setProjects(ps);
    setActiveProject(cur => cur && ps.some(p => p.id === cur.id) ? cur : (ps[0] ?? null));
  }, []);

  useEffect(() => { loadProjects(); loadProjectReviewCounts(); }, [loadProjects, loadProjectReviewCounts]);

  // 切项目：加载本地分支 + 当前运行中的分支预览
  useEffect(() => {
    if (!activeProject) { setBranches([]); setBranchPreviews([]); return; }
    const pid = activeProject.id;
    listLocalBranches(pid).then(b => { if (activeProject?.id === pid) setBranches(b); }).catch(() => setBranches([]));
    listBranchPreviews(pid).then(b => { if (activeProject?.id === pid) setBranchPreviews(b); }).catch(() => setBranchPreviews([]));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeProject]);

  // 只要有分支预览在跑就持续轮询：既等 starting→running，也及时发现进程退出
  //（用户手动关掉 Tauri 预览窗口 → tauri dev 进程结束 → 后端 try_wait 判定已停止 → 这里同步移除）。
  // 依赖布尔量而非数组，避免每次拉取都重建定时器；列表清空后自动停轮询。
  const hasBranchPreviews = branchPreviews.length > 0;
  useEffect(() => {
    if (!activeProject || !hasBranchPreviews) return;
    const pid = activeProject.id;
    const id = setInterval(() => {
      listBranchPreviews(pid).then(b => { if (activeProject?.id === pid) setBranchPreviews(b); }).catch(() => {});
    }, 2500);
    return () => clearInterval(id);
  }, [hasBranchPreviews, activeProject]);

  const loadList = useCallback(async (projectId: string) => {
    const [allCrs, allIssues] = await Promise.all([
      listChangeRequests(projectId),
      listIssues(projectId),
    ]);
    setCrs(allCrs);
    setPendingIssues(allIssues.filter(i => i.status === 'pending_review_1'));
    setIssueTitles(Object.fromEntries(allIssues.map(i => [i.id, i.title])));
    setIssuesById(Object.fromEntries(allIssues.map(i => [i.id, i])));
    setLoadedProjectId(projectId);
  }, []);

  useEffect(() => { if (activeProject) loadList(activeProject.id); }, [activeProject, loadList]);

  // 默认选中 / 校验当前选择仍有效（target 导航时跳过，交由 target effect 处理）
  useEffect(() => {
    if (target) return;
    if (loadedProjectId !== activeProject?.id) return;
    const stillValid = sel && (sel.kind === 'cr' ? crs.some(c => c.id === sel.id) : pendingIssues.some(i => i.id === sel.id));
    if (stillValid) return;
    if (pendingIssues.length) setSel({ kind: 'issue', id: pendingIssues[0].id });
    else if (crs.length) setSel({ kind: 'cr', id: sortedCrs(crs)[0].id });
    else setSel(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [crs, pendingIssues, loadedProjectId, activeProject, target]);

  // 跨页跳转：切到目标项目
  useEffect(() => {
    if (!target) return;
    if (!projects.length) return;  // 等项目列表就绪
    const proj = projects.find(p => p.id === target.projectId);
    if (!proj) { onTargetConsumed(); return; }  // 目标项目不在产，放弃跳转
    if (proj.id !== activeProject?.id) setActiveProject(proj);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target, projects]);

  // 跨页跳转：目标项目数据就绪后，解析为 issue 或 cr 选中
  useEffect(() => {
    if (!target) return;
    if (loadedProjectId !== target.projectId) return;
    const issue = pendingIssues.find(i => i.id === target.issueId);
    if (issue) setSel({ kind: 'issue', id: issue.id });
    else {
      const cr = crs.find(c => c.issue_id === target.issueId);
      if (cr) setSel({ kind: 'cr', id: cr.id });
    }
    setDecided(null);
    onTargetConsumed();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target, loadedProjectId, crs, pendingIssues]);

  useEffect(() => {
    if (!activeCr) {
      setCrPreview(null);  // clear stale preview when no CR is selected
      return;
    }
    const crId = activeCr;
    const reqId = ++loadReqRef.current;
    setCrLoading(true);
    setDecided(null);
    setGrade(null);
    setAdvice('');
    setSession(null);   // 清掉上一份（含上一版本）报告，避免显示过期内容
    setDiff('');        // diff='' 时视图显示「加载中…」，重拉后替换
    autoOpenRef.current = false;  // 切换 CR 时取消上一条未完成的自动打开
    setTimeout(() => adviceRef.current?.focus(), 120);

    (async () => {
      const [s, d, g, pv] = await Promise.all([
        getWorktreeSession(crId),
        getCodeDiff(crId),
        getCrGrade(crId).catch(() => null),
        getCrPreview(crId).catch(() => null),
      ]);
      if (loadReqRef.current !== reqId) return;
      setSession(s);
      setCrPreview(pv);
      setDiff(d);
      setGrade(g);
      setCrLoading(false);
    })();
    // activeCrUpdatedAt 入依赖：同一 CR 修改/重新执行后 updated_at 变化即重新拉取
  }, [activeCr, activeProject, activeCrUpdatedAt]);

  // Poll the CR preview while its dev server is spinning up.
  useEffect(() => {
    if (crPreview?.status !== 'starting') return;
    const crId = activeCr;
    if (!crId) return;
    const id = setInterval(() => {
      getCrPreview(crId).then(p => {
        if (!loadReqRef.current || activeCr !== crId) return;
        setCrPreview(p);
        // web 预览就绪：若用户点的是「启动并打开浏览器」，此刻自动打开
        if (p.status === 'running' && p.url && autoOpenRef.current) {
          autoOpenRef.current = false;
          openUrl(p.url).catch(() => {});
        }
      }).catch(() => {});
    }, 2000);
    return () => clearInterval(id);
  }, [crPreview?.status, activeCr]);

  // 审核 1：选中 Issue 时加载其分析结果
  useEffect(() => {
    if (!activeIssueId) { setIssueAnalysis(null); return; }
    const issueId = activeIssueId;
    setAnalysisLoading(true);
    setDecided(null);
    setAdvice('');
    getIssueAnalysis(issueId)
      .then(a => { if (activeIssueId === issueId) setIssueAnalysis(a); })
      .catch(() => { if (activeIssueId === issueId) setIssueAnalysis(null); })
      .finally(() => setAnalysisLoading(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeIssueId]);

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const debounced = () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = null;
        if (activeProject) loadList(activeProject.id);
        loadProjectReviewCounts();
      }, 500);
    };
    let unlisten: (() => void) | undefined;
    listen('AutoForge://event', debounced).then(fn => { unlisten = fn; });
    return () => { if (timer) clearTimeout(timer); unlisten?.(); };
  }, [activeProject, loadList, loadProjectReviewCounts]);

  const doReview = async (decision: 'approved' | 'revision' | 'rejected') => {
    if (!activeCr || submitting) return;
    setSubmitting(true);
    try {
      await review2(activeCr, { decision, suggestions: advice || undefined });
      setDecided(decision);
      if (activeProject) await loadList(activeProject.id);
      await loadProjectReviewCounts();
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    } finally { setSubmitting(false); }
  };

  // 失败需求闭环：重新执行（回到编码队列）。
  const doRetry = async () => {
    if (!activeCr || submitting) return;
    setSubmitting(true);
    try {
      await retryChangeRequest(activeCr);
      if (activeProject) await loadList(activeProject.id);
      await loadProjectReviewCounts();
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    } catch (e) {
      showError('重新执行失败：' + String(e));
    } finally { setSubmitting(false); }
  };

  // 失败需求闭环：彻底删除需求及其执行数据。
  const doDelete = async () => {
    if (!activeCr || submitting) return;
    if (!window.confirm('确定删除该需求？将一并清除其执行产物、变更请求与原始需求，且不可恢复。')) return;
    setSubmitting(true);
    try {
      await deleteChangeRequest(activeCr);
      setSel(null);
      if (activeProject) await loadList(activeProject.id);
      await loadProjectReviewCounts();
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    } catch (e) {
      showError('删除失败：' + String(e));
    } finally { setSubmitting(false); }
  };

  // 审核 1：批准 → 创建 CR 进入编码；拒绝 → 归档（后端按设计返回 Err）。
  const doReview1 = async (decision: 'approved' | 'rejected') => {
    if (!activeIssueId || submitting) return;
    setSubmitting(true);
    let newCr: ChangeRequest | null = null;
    try {
      newCr = await review1(activeIssueId, { decision, suggestions: advice || undefined });
    } catch { /* reject 按设计返回 Err，状态已更新 */ }
    finally {
      setDecided(decision);
      if (activeProject) await loadList(activeProject.id);
      await loadProjectReviewCounts();
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
      // 批准后自动跳到新生成的 CR
      if (decision === 'approved' && newCr) setSel({ kind: 'cr', id: newCr.id });
      setSubmitting(false);
    }
  };

  // 2. Enter 发送（Shift+Enter 换行）
  const onAdviceKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      if (canRevise && !submitting) doReview('revision');
    }
  };

  // 启动选定分支（worktree 隔离，多分支可并行）；web 就绪后自动开浏览器
  const doStartBranch = useCallback(async (branch: string) => {
    if (!activeProject) return;
    const pid = activeProject.id;
    try {
      const st = await startBranchPreview(pid, branch);
      setBranchPreviews(prev => [...prev.filter(p => p.branch !== branch), st].sort((a, b) => a.branch.localeCompare(b.branch)));
      // 启动即打开实时日志窗口，便于观察编译/启动进度
      setLogModal({ title: `启动日志 · ${branch}`, sig: `branch:${pid}:${branch}`, load: () => getBranchPreviewLog(pid, branch) });
      if (st.kind === 'web' && st.url) {
        // 轮询直到可达再开浏览器，避免打开尚未就绪的空白页
        const open = async (tries: number) => {
          const list = await listBranchPreviews(pid).catch(() => []);
          const cur = list.find(p => p.branch === branch);
          if (cur?.status === 'running' && cur.url) { openUrl(cur.url).catch(() => {}); return; }
          if (tries > 0) setTimeout(() => open(tries - 1), 1500);
        };
        open(20);
      }
    } catch (e) { showError('启动失败：' + String(e)); }
  }, [activeProject, showError]);

  const doStopBranch = useCallback(async (branch: string) => {
    if (!activeProject) return;
    try {
      await stopBranchPreview(activeProject.id, branch);
      setBranchPreviews(prev => prev.filter(p => p.branch !== branch));
    } catch (e) { showError('停止失败：' + String(e)); }
  }, [activeProject, showError]);

  const showBranchLog = useCallback((branch: string) => {
    if (!activeProject) return;
    const pid = activeProject.id;
    setLogModal({ title: `启动日志 · ${branch}`, sig: `branch:${pid}:${branch}`, load: () => getBranchPreviewLog(pid, branch) });
  }, [activeProject]);

  // web 项目：在 worktree 启动 dev server，就绪后自动打开浏览器（starting 时交给轮询补打开）
  const doStartCrPreview = useCallback(async () => {
    if (!activeCr) return;
    try {
      const st = await startCrPreview(activeCr);
      setCrPreview(st);
      if (st.status === 'running' && st.url) openUrl(st.url).catch(() => {});
      else if (st.status === 'starting') autoOpenRef.current = true;
    } catch (e) { showError('启动预览失败：' + String(e)); }
  }, [activeCr]);

  const doStopCrPreview = useCallback(async () => {
    if (!activeCr) return;
    try {
      await stopCrPreview(activeCr);
      setCrPreview(p => p ? { ...p, status: 'stopped', url: null } : null);
    } catch (e) { showError('停止失败：' + String(e)); }
  }, [activeCr]);

  const doLaunchCrApp = useCallback(async () => {
    if (!activeCr) return;
    try { await launchCrApp(activeCr); }
    catch (e) { showError('启动桌面应用失败：' + String(e)); }
  }, [activeCr, showError]);

  const showCrPreviewLog = useCallback(() => {
    if (!activeCr) return;
    const id = activeCr;
    setLogModal({ title: '预览日志 · 本次改动', sig: `cr:${id}`, load: () => getCrPreviewLog(id) });
  }, [activeCr]);

  const cr = crs.find(c => c.id === activeCr);
  // 「本次改动」预览：worktree 存在才可启动（合并后 no_session → 隐藏预览按钮）
  const showCrPreview = !!crPreview && crPreview.kind !== 'none' && crPreview.status !== 'no_session';
  const selectedIssue = activeIssueId ? pendingIssues.find(i => i.id === activeIssueId) : undefined;
  const report = session?.report_content ? parseReport(session.report_content) : null;
  const hunks = diff ? parseDiff(diff) : [];
  const canRevise = cr?.status === 'pending_review_2' && !decided;

  // 「本次改动」预览的启动动作：web → 起 dev server 并自动开浏览器；tauri → 直接启动桌面程序
  const renderCrLaunch = () => {
    if (!crPreview || crPreview.kind === 'none') return null;
    const { kind, status, url, can_launch_app } = crPreview;
    if (status === 'no_session') return null;
    if (status === 'starting') {
      return (
        <button className="btn btn-sm" disabled>
          <span className="dot amber" style={{ marginRight: 4 }} />启动中…
        </button>
      );
    }
    if (kind === 'tauri') {
      // tauri：直接启动桌面程序（可访问完整 IPC），无需 iframe
      return (
        <>
          <button className="btn btn-sm" disabled={!can_launch_app} onClick={doLaunchCrApp}>
            <Icon name="box" size={14} />启动 Tauri 程序
          </button>
          <button className="btn btn-sm btn-ghost" onClick={showCrPreviewLog} title="查看启动日志">
            <Icon name="log" size={14} />
          </button>
        </>
      );
    }
    // web：运行中显示「打开浏览器 / 停止」，否则「启动并打开浏览器」
    if (status === 'running' && url) {
      return (
        <>
          <button className="btn btn-sm" onClick={() => openUrl(url).catch(() => {})}>
            <Icon name="external" size={14} />打开浏览器
          </button>
          <button className="btn btn-sm btn-ghost" onClick={doStopCrPreview} title="停止预览服务">
            <Icon name="x" size={14} />
          </button>
          <button className="btn btn-sm btn-ghost" onClick={showCrPreviewLog} title="查看启动日志">
            <Icon name="log" size={14} />
          </button>
        </>
      );
    }
    return (
      <>
        <button className="btn btn-sm" onClick={doStartCrPreview}>
          <Icon name="play" size={14} />启动并打开浏览器
        </button>
        <button className="btn btn-sm btn-ghost" onClick={showCrPreviewLog} title="查看启动日志">
          <Icon name="log" size={14} />
        </button>
      </>
    );
  };

  return (
    <div className="audit-page">
      <div className="audit-top audit-head-main" style={{ height: 56 }}>
        <div className="eyebrow" style={{ fontSize: 'var(--text-heading)' }}>
          <span className="en">AUDIT</span><span className="cn">· 功能审计</span>
        </div>
        {activeProject && (
          <BranchLauncher
            branches={branches} branchPreviews={branchPreviews}
            onStart={doStartBranch} onStop={doStopBranch} onShowLog={showBranchLog}
          />
        )}
      </div>

      <div className="audit-workspace">
        {/* 1. 左侧列表 + 第一个拖拽分割线 */}
        <AuditList
          projects={projects} activeProject={activeProject}
          setActiveProject={p => { setActiveProject(p); setSel(null); }}
          projectReviewCounts={projectReviewCounts} crs={crs} pendingIssues={pendingIssues} issueTitles={issueTitles} sel={sel}
          onSelectCr={id => { setSel({ kind: 'cr', id }); setDecided(null); }}
          onSelectIssue={id => { setSel({ kind: 'issue', id }); setDecided(null); }}
          onOpenIntake={() => setIntakeOpen(true)}
          width={listWidth}
        />
        <ResizeHandle onDrag={dx => setListWidth(w => Math.max(180, Math.min(520, w + dx)))} />

        <div className="content">
          {selectedIssue ? (
            <IssueReviewView
              issue={selectedIssue} analysis={issueAnalysis} analysisLoading={analysisLoading}
              submitting={submitting} decided={decided}
              advice={advice} setAdvice={setAdvice} onDecide={doReview1}
            />
          ) : cr ? (
            <>
              {/* 顶部标题栏 */}
              <div className="audit-top">
                <div>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <span className="req-id" style={{ fontSize: 'var(--text-control)' }}>{cr.id.slice(0, 10)}</span>
                    <span style={{ fontWeight: 700, fontSize: 'var(--text-title)' }}>{issueTitles[cr.issue_id] || 'Change Request'}</span>
                    {session && <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)' }}>迭代 {session.iteration_count} 轮</span>}
                    {grade && <span className={'chip ' + (grade.tier === 'T3' ? 'red' : grade.tier === 'T2' ? 'amber' : grade.tier === 'T1' ? 'blue' : 'green')} title={grade.rationale}>风险 {grade.tier} · {grade.change_class}</span>}
                  </div>
                  <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 2 }}>
                    {STATUS_LABEL[cr.status] ?? cr.status} · {new Date(cr.updated_at).toLocaleString('zh')}
                  </div>
                </div>
                <div className="audit-decide">
                  {FAILED_STATUSES.includes(cr.status)
                    ? <>
                        <span className={'chip ' + (STATUS_COLOR[cr.status] ?? 'red')} style={{ padding: '7px 14px', fontSize: 'var(--text-control)' }}>
                          <Icon name="alert" size={14} />{STATUS_LABEL[cr.status] ?? cr.status}
                        </span>
                        <button className="btn btn-danger" onClick={doDelete} disabled={submitting}><Icon name="trash" size={15} />删除需求</button>
                        <button className="btn btn-primary" onClick={doRetry} disabled={submitting}><Icon name="refresh" size={15} />重新执行</button>
                      </>
                    : cr.status !== 'pending_review_2'
                    ? <span className={'chip ' + (STATUS_COLOR[cr.status] ?? '')} style={{ padding: '7px 14px', fontSize: 'var(--text-control)' }}>
                        {STATUS_LABEL[cr.status] ?? cr.status}
                      </span>
                    : decided
                      ? <span className={'chip ' + (decided === 'approved' ? 'green' : decided === 'rejected' ? 'red' : 'amber')} style={{ padding: '7px 14px', fontSize: 'var(--text-control)' }}>
                          <Icon name={decided === 'approved' ? 'check' : decided === 'rejected' ? 'x' : 'refresh'} size={14} />
                          {decided === 'approved' ? '已批准 · 合并到 dev' : decided === 'rejected' ? '已拒绝' : '已退回 · 重新执行'}
                        </span>
                      : <>
                          <button className="btn btn-danger" onClick={() => doReview('rejected')} disabled={submitting}><Icon name="x" size={15} />拒绝</button>
                          <button className="btn btn-primary" onClick={() => doReview('approved')} disabled={submitting}><Icon name="check" size={15} />批准合并</button>
                        </>}
                </div>
              </div>

              {/* 1. 内容区三栏：left + resize + right */}
              <div className={`audit-split${crLoading ? ' cr-loading' : ''}`}>
                {/* 中栏：报告 / diff */}
                <div className="audit-left">
                  <div className="diff-tabbar">
                    <div className="seg">
                      <button className={tab === 'report' ? 'on' : ''} onClick={() => setTab('report')}>实现报告</button>
                      <button className={tab === 'diff' ? 'on' : ''} onClick={() => setTab('diff')}>代码 Diff</button>
                    </div>
                    {tab === 'report' && issuesById[cr.issue_id] && (
                      <button className="btn btn-sm" style={{ marginLeft: 'auto' }} onClick={() => setOrigReqOpen(o => !o)}>
                        <Icon name={origReqOpen ? 'eye-off' : 'eye'} size={13} />{origReqOpen ? '收起' : '查看'}需求原文
                      </button>
                    )}
                    {tab === 'diff' && (
                      <div className="seg" style={{ marginLeft: 'auto' }}>
                        <button className={diffMode === 'unified' ? 'on' : ''} onClick={() => setDiffMode('unified')}>
                          <Icon name="rows" size={13} style={{ verticalAlign: -2, marginRight: 4 }} />统一
                        </button>
                        <button className={diffMode === 'split' ? 'on' : ''} onClick={() => setDiffMode('split')}>
                          <Icon name="columns" size={13} style={{ verticalAlign: -2, marginRight: 4 }} />分栏
                        </button>
                      </div>
                    )}
                  </div>
                  <div className="diff-viewport scroll">
                    {tab === 'report' ? (
                      <div className="report">
                        {origReqOpen && (() => {
                          const oi = issuesById[cr.issue_id];
                          if (!oi) return null;
                          return (
                            <div className="panel" style={{ marginBottom: 12, padding: '12px 14px' }}>
                              <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap', marginBottom: 8 }}>
                                <span style={{ fontWeight: 700, fontSize: 'var(--text-body)' }}>{oi.title}</span>
                                <span className={'chip ' + (SEV_COLOR[oi.category] || 'blue')} style={{ fontSize: 'var(--text-micro)' }}>{oi.category}</span>
                                <span className={'chip ' + (SEV_COLOR[oi.severity] || '')} style={{ fontSize: 'var(--text-micro)' }}>{oi.severity}</span>
                                <span style={{ marginLeft: 'auto', fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{oi.source_type} · {new Date(oi.created_at).toLocaleString('zh')}</span>
                              </div>
                              <p style={{ margin: 0, whiteSpace: 'pre-line', fontSize: 'var(--text-control)', color: 'var(--text-2)', lineHeight: 'var(--leading-normal)' }}>{oi.description || '（无描述）'}</p>
                            </div>
                          );
                        })()}
                        {FAILED_STATUSES.includes(cr.status) ? (
                          <div style={{ background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderLeft: '3px solid var(--red)', borderRadius: 10, padding: '14px 16px' }}>
                            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10, color: 'var(--red)', fontWeight: 700, fontSize: 'var(--text-body)' }}>
                              <Icon name="alert" size={18} />{STATUS_LABEL[cr.status] ?? '执行失败'}原因
                            </div>
                            <pre style={{ margin: 0, whiteSpace: 'pre-wrap', wordBreak: 'break-word', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-2)', lineHeight: 1.6 }}>
                              {crLoading ? '加载中…' : (session?.report_content || '未捕获到失败详情，请重新执行或查看日志。')}
                            </pre>
                            <div style={{ marginTop: 12, fontSize: 'var(--text-label)', color: 'var(--text-3)' }}>
                              可使用右上角「重新执行」重试，或「删除需求」清除该条异常数据。
                            </div>
                          </div>
                        ) : (<>
                        {session && (session.iteration_count ?? 0) >= 3 && (
                          <div className="iter-warn"><Icon name="alert" size={20} /><div>已迭代 <b>{session.iteration_count}</b> 轮（软上限 3）。建议手动介入或重新描述需求。</div></div>
                        )}
                        {report ? (
                          <>
                            <h2><Icon name="zap" size={18} style={{ color: 'var(--ember)' }} />改动摘要</h2>
                            <p>{report.summary}</p>
                            {report.files.length > 0 && (
                              <><h2><Icon name="file" size={18} style={{ color: 'var(--blue)' }} />修改文件</h2>
                                <div>{report.files.map((f, i) => (
                                  <span className="file-pill" key={i}><Icon name="file" size={13} />{f.name}<span className="add">+{f.add}</span>{f.del > 0 && <span className="del">-{f.del}</span>}</span>
                                ))}</div></>
                            )}
                            {report.testsSection && (
                              <><h2><Icon name="flask" size={18} style={{ color: 'var(--green)' }} />测试情况</h2><p style={{ whiteSpace: 'pre-line' }}>{report.testsSection}</p></>
                            )}
                            {report.risk && (
                              <><h2><Icon name="shield" size={18} style={{ color: 'var(--violet)' }} />潜在风险</h2><p>{report.risk}</p></>
                            )}
                          </>
                        ) : (
                          <div className="empty-compact" style={{ padding: '20px 0' }}>{session ? '报告内容为空' : '加载中…'}</div>
                        )}
                        </>)}
                      </div>
                    ) : (
                      <div className="diff">
                        {hunks.length === 0
                          ? <div className="empty-compact" style={{ padding: '20px 22px' }}>{crLoading ? '加载中…' : 'Diff 为空或 worktree 不存在'}</div>
                          : hunks.map((h, hi) => (
                            <div key={hi}>
                              <div className="diff-toolbar">
                                <Icon name="file" size={15} style={{ color: 'var(--text-3)' }} />
                                <span className="diff-file">{h.file}</span>
                              </div>
                              <div className="diff-hunk">{h.hunk}</div>
                              {diffMode === 'unified'
                                ? h.lines.map((l, i) => (
                                  <div key={i} className={'diff-line ' + (l.t === 'add' ? 'add' : l.t === 'del' ? 'del' : '')}>
                                    <span className="gut">{l.n1}</span><span className="gut">{l.n2}</span>
                                    <span className="code">{l.t === 'add' ? '+ ' : l.t === 'del' ? '- ' : '  '}{l.code}</span>
                                  </div>
                                ))
                                : <div className="diff-split-wrap">
                                    <div>{h.lines.filter(l => l.t !== 'add').map((l, i) => (
                                      <div key={i} className={'diff-line ' + (l.t === 'del' ? 'del' : '')}>
                                        <span className="gut">{l.n1}</span><span className="code">{l.code}</span>
                                      </div>
                                    ))}</div>
                                    <div>{h.lines.filter(l => l.t !== 'del').map((l, i) => (
                                      <div key={i} className={'diff-line ' + (l.t === 'add' ? 'add' : '')}>
                                        <span className="gut">{l.n2}</span><span className="code">{l.code}</span>
                                      </div>
                                    ))}</div>
                                  </div>
                              }
                            </div>
                          ))}
                      </div>
                    )}
                  </div>

                  {/* 底部悬浮 dock：左 = 本次改动预览启动；右 = 管理员建议 + 修改 */}
                  <div className="audit-dock">
                    {showCrPreview && (
                      <div className="dock-preview">
                        <span className="dock-label">本次改动预览</span>
                        <div className="dock-preview-actions">{renderCrLaunch()}</div>
                      </div>
                    )}
                    <div className="dock-advice">
                      <span className="dock-label">管理员建议 → Claude Code</span>
                      <div className="dock-advice-row">
                        <textarea
                          ref={adviceRef}
                          value={advice}
                          onChange={e => setAdvice(e.target.value)}
                          onKeyDown={onAdviceKeyDown}
                          placeholder={canRevise ? '输入修改意见，Enter 发送，Shift+Enter 换行…' : '输入备注（只读状态不会提交）…'}
                        />
                        <button className="btn btn-sm" onClick={() => doReview('revision')}
                          disabled={!canRevise || submitting}
                          title={canRevise ? '提交修改意见，退回重新执行' : '仅「待代码审核」状态可提交修改'}>
                          <Icon name="refresh" size={14} />修改
                        </button>
                      </div>
                    </div>
                  </div>
                </div>
              </div>
            </>
          ) : (
            <div className="empty" style={{ flex: 1 }}><Icon name="audit" /><div>选择一个需求查看详情</div></div>
          )}
        </div>
      </div>

      {intakeOpen && activeProject && (
        <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 220 }}>
          <div style={{ width: 720, maxHeight: 'min(800px, calc(100vh - 32px))', background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', overflow: 'hidden', display: 'flex', flexDirection: 'column' }} onClick={e => e.stopPropagation()}>
            <div style={{ padding: '18px 20px 14px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
              <div className="eyebrow" style={{ fontSize: 'var(--text-section)' }}>
                <span className="cn">需求入口</span>
                <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginLeft: 8, fontFamily: 'var(--font-sans)', letterSpacing: 0, textTransform: 'none' }}>{activeProject.name}</span>
              </div>
              <button className="icon-btn" onClick={() => setIntakeOpen(false)}><Icon name="x" size={18} /></button>
            </div>
            <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
              <IntakePanel key={activeProject.id} projectId={activeProject.id} />
            </div>
          </div>
        </div>
      )}

      {logModal && (
        <LiveLogModal key={logModal.sig} title={logModal.title} load={logModal.load} onClose={() => setLogModal(null)} />
      )}

      <Toast data={toast} onClose={() => setToast(null)} />
    </div>
  );
}
