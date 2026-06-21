import React, { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { listen } from '@tauri-apps/api/event';
import Icon from '../components/Icon';
import Toast, { type ToastData } from '../components/Toast';
import IntakePanel from '../components/IntakePanel';
import { ConfirmModal } from '../components/ProjectDialogs';
import { ReaderToc } from '../components/ReaderToc';
import { toggleMaximizeOnDoubleClick } from '../lib/window';
import { fmtShort, fmtFull } from '../utils/datetime';
import {
  listActiveProjects, listChangeRequests, listChangeRequestsPage, getChangeRequestByIssue, getWorktreeSession, getCodeDiff, review2, review2Batch, getCrGrade,
  retryChangeRequest, deleteChangeRequest, retryAnalysis, reanalyzeWithFeedback,
  openUrl, getIssue, listIssuesPage, listIssueStatuses, listIssuesByStatuses, listIssueTitles,
  getIssueAnalysis, review1, review1Batch, parseAnalysisSpec, updateIssueAcceptance, refineTriage, rejectIssues,
  getCrPreview, startCrPreview, stopCrPreview, launchCrApp, getCrPreviewLog,
  listLocalBranches, startBranchPreview, listBranchPreviews, stopBranchPreview, getBranchPreviewLog,
  getMergeConflict, retryMerge, aiResolveMergeConflict, revertChangeRequest, getCustomMergeMessageEnabled,
  getConflictDetail, resolveConflictManually, openConflictWorkspace,
  type Project, type ChangeRequest, type WorktreeSession, type CrGrade,
  type CrPreviewStatus, type Issue, type IssueAnalysis, type IssueAnalysisSpec,
  type BranchInfo, type BranchPreviewStatus, type MergeConflictView,
  type ConflictDetail,
} from '../services';

type Sel = { kind: 'issue' | 'cr'; id: string };

// 复制完整需求编号到剪贴板，附带短暂的成功反馈
function CopyIdButton({ value, title = '复制编号' }: { value: string; title?: string }) {
  const [copied, setCopied] = useState(false);
  const doCopy = async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch { /* 剪贴板不可用时静默忽略 */ }
  };
  return (
    <button
      className="icon-btn btn-sm"
      style={{ width: 24, height: 24, padding: 0 }}
      onClick={doCopy}
      title={copied ? '已复制' : title}
    >
      <Icon name={copied ? 'check' : 'copy'} size={13} style={copied ? { color: 'var(--green)' } : undefined} />
    </button>
  );
}

const STATUS_LABEL: Record<string, string> = {
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
// Failed states float to the top so abnormal requirements are easy to find and resolve.
const STATUS_ORDER = ['execution_failed', 'merge_failed', 'merge_conflict', 'pending_code_review', 'executing', 'pending_execution', 'pending_merge', 'pending_issue_review', 'no_change_needed', 'merged', 'rejected'];

// Stuck/abnormal CR states that the user can recover (retry) or remove (delete).
const FAILED_STATUSES = ['execution_failed', 'merge_failed', 'merge_conflict'];
// Ran cleanly but ended without a diff — recoverable via retry/delete like a
// failure, but shown neutrally (info) rather than as a red error.
const NO_CHANGE_STATUSES = ['no_change_needed'];
// Either kind exposes the same retry/delete affordances in the decision bar.
const RECOVERABLE_STATUSES = [...FAILED_STATUSES, ...NO_CHANGE_STATUSES];

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
  let oldPath = '';
  let curHunk: Hunk | null = null;
  let n1 = 0, n2 = 0;
  for (const line of raw.split('\n')) {
    if (line.startsWith('diff --git ')) { curFile = ''; oldPath = ''; continue; }
    // 新增文件的 `--- ` 行是 `/dev/null`，真实路径在 `+++ b/...` 行；
    // 删除文件反之。优先用新路径，删除时回退旧路径，避免显示 /dev/null。
    if (line.startsWith('--- ')) { oldPath = line.slice(4).replace(/^a\//, ''); curFile = oldPath; continue; }
    if (line.startsWith('+++ ')) {
      const newPath = line.slice(4).replace(/^b\//, '');
      curFile = newPath === '/dev/null' ? oldPath : newPath;
      continue;
    }
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

// 预览进程生命周期阶段（喂给日志窗口头部状态灯，让用户分清「仍在启动」与「已退出/报错」）。
type LogPhase = 'starting' | 'running' | 'stopped' | null;
const LOG_PHASE_META: Record<'starting' | 'running' | 'stopped', { dot: string; label: string }> = {
  starting: { dot: 'amber', label: '启动中…' },
  running: { dot: 'green', label: '运行中' },
  stopped: { dot: 'red', label: '进程已退出 · 见日志末尾确认是否报错' },
};

// 实时日志窗口：定时拉取、按行高亮、跟随底部自动滚动（用户上滚则暂停跟随）。
// `phase` 由父组件的状态轮询喂入，使头部状态灯能区分「启动中 / 运行中 / 已退出」——
// 否则进程崩溃后日志只是停更，用户无从判断是仍在编译还是已经报错退出。
function LiveLogModal({ title, load, phase, onClose }: {
  title: string; load: () => Promise<string>; phase?: () => LogPhase; onClose: () => void;
}) {
  const [raw, setRaw] = useState('');
  const [livePhase, setLivePhase] = useState<LogPhase>('starting');
  const bodyRef = useRef<HTMLDivElement>(null);
  const followRef = useRef(true);
  const loadRef = useRef(load);
  loadRef.current = load;
  const phaseRef = useRef(phase);
  phaseRef.current = phase;
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  // 仅挂载时启动轮询（组件按 sig key 重挂载切换目标），避免父组件 re-render 重置定时器
  useEffect(() => {
    let alive = true;
    const tick = () => {
      loadRef.current().then(t => { if (alive) setRaw(t); }).catch(() => {});
      if (alive && phaseRef.current) setLivePhase(phaseRef.current());
    };
    tick();
    const id = window.setInterval(tick, 1000);
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') closeRef.current(); };
    window.addEventListener('keydown', onKey);
    return () => { alive = false; window.clearInterval(id); window.removeEventListener('keydown', onKey); };
  }, []);

  const lines = useMemo(() => parseLogLines(raw), [raw]);
  const meta = livePhase ? LOG_PHASE_META[livePhase] : null;

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
          <span className={'dot ' + (meta?.dot ?? 'green')} style={{ flexShrink: 0 }} />
          <span className="eyebrow" style={{ fontSize: 'var(--text-section)' }}><span className="cn">{title}</span></span>
          <span style={{ marginLeft: 'auto', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: livePhase === 'stopped' ? 'var(--red)' : 'var(--text-faint)' }}>
            {raw ? `${lines.length} 行 · ` : ''}{meta ? meta.label : '实时跟随'}
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

function BranchLauncher({ branches, branchPreviews, onStart, onStop, onShowLog, onOpenIntake, onOpenLedger, showMerged, onToggleMerged, mergedCount }: {
  branches: BranchInfo[]; branchPreviews: BranchPreviewStatus[];
  onStart: (b: string) => void; onStop: (b: string) => void; onShowLog: (b: string) => void;
  onOpenIntake: () => void; onOpenLedger: () => void;
  showMerged: boolean; onToggleMerged: () => void; mergedCount: number;
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
      <button
        className={'icon-btn' + (showMerged ? ' on' : '')}
        style={{ flexShrink: 0, position: 'relative' }}
        onClick={onToggleMerged}
        title={showMerged ? `隐藏已合并需求（${mergedCount}）` : `显示已合并需求（${mergedCount}）`}>
        <Icon name={showMerged ? 'eye' : 'eye-off'} size={16} />
        {!showMerged && mergedCount > 0 && (
          <span className="dot green" style={{ position: 'absolute', top: 4, right: 4 }} />
        )}
      </button>
      <button className="icon-btn" style={{ flexShrink: 0 }} onClick={onOpenLedger} title="全量需求总账（全屏查看所有状态需求）">
        <Icon name="list" size={16} />
      </button>
      <button className="icon-btn" style={{ flexShrink: 0 }} onClick={onOpenIntake} title="需求入口（提交 / 同步 / 扫描 / 批量导入）">
        <Icon name="download" size={16} />
      </button>
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
  onSelectCr, onSelectIssue, onOpenLedger, onBatchApprove, onBatchApproveCrs, gate,
  width, hasMoreMerged, mergedLoading, onLoadMoreMerged }: {
  projects: Project[]; activeProject: Project | null; setActiveProject: (p: Project) => void;
  projectReviewCounts: Record<string, number>; crs: ChangeRequest[]; pendingIssues: Issue[];
  issueTitles: Record<string, string>; sel: Sel | null;
  onSelectCr: (id: string) => void; onSelectIssue: (id: string) => void; onOpenLedger: () => void;
  // 批量需求审核：通过选中的待审核需求，返回 Promise 供调用方等待刷新。
  onBatchApprove: (ids: string[]) => Promise<void> | void;
  // 批量代码审核：通过选中的待代码审核 变更请求（各自排队合并）。
  onBatchApproveCrs: (ids: string[]) => Promise<void> | void;
  // 当前审核闸口：'issue' 只显示待需求审核，'code' 只显示变更请求各态。
  gate: 'issue' | 'code';
  width: number;
  // 已合并 CR 分批加载：是否还有下一页 / 正在加载 / 触发加载下一页。
  hasMoreMerged: boolean; mergedLoading: boolean; onLoadMoreMerged: () => void;
}) {
  const [open, setOpen] = useState(false);
  const menuRef = React.useRef<HTMLDivElement>(null);
  // 已合并 CR 滚动加载：滚动容器 + 触底哨兵。
  const listScrollRef = React.useRef<HTMLDivElement>(null);
  const mergedSentinelRef = React.useRef<HTMLDivElement>(null);
  // 批量审核选区：审核需求闸作用于 pending_issue_review 需求，审核代码闸作用于 pending_code_review CR。
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirmBatch, setConfirmBatch] = useState(false);
  const [batching, setBatching] = useState(false);

  // 列表模糊过滤：按标题或需求编号（短/全 id）筛选当前闸口列表。
  const [search, setSearch] = useState('');
  const filteredIssues = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return pendingIssues;
    return pendingIssues.filter(i => i.title.toLowerCase().includes(q) || i.id.toLowerCase().includes(q));
  }, [pendingIssues, search]);
  const filteredCrs = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return crs;
    return crs.filter(r =>
      (issueTitles[r.issue_id] || '').toLowerCase().includes(q) ||
      r.id.toLowerCase().includes(q) ||
      r.issue_id.toLowerCase().includes(q));
  }, [crs, issueTitles, search]);
  // 切换闸口时清空搜索，避免跨闸口残留筛选词。
  useEffect(() => { setSearch(''); }, [gate]);

  // 仅 pending_issue_review 可批量通过（analysis_failed 需先重分析，不可直接过审）；批量仅作用于当前筛选可见集。
  const selectablePending = useMemo(() => filteredIssues.filter(i => i.status === 'pending_issue_review'), [filteredIssues]);
  // 仅 pending_code_review 的 CR 可批量通过（执行中/已合并等态不可直接过审）。
  const selectableCrs = useMemo(() => filteredCrs.filter(r => r.status === 'pending_code_review'), [filteredCrs]);
  // 当前闸口下可批量操作的目标 id 集合。
  const selectableIds = useMemo(
    () => (gate === 'issue' ? selectablePending.map(i => i.id) : selectableCrs.map(r => r.id)),
    [gate, selectablePending, selectableCrs],
  );
  // 列表刷新后剔除已离开队列的选中项，避免对幽灵 id 批量操作。
  useEffect(() => {
    setSelected(prev => {
      const valid = new Set(selectableIds);
      const next = new Set([...prev].filter(id => valid.has(id)));
      return next.size === prev.size ? prev : next;
    });
  }, [selectableIds]);
  // 切换闸口时清空批量选区，避免跨闸口选区残留与底部操作条错位。
  useEffect(() => { setSelected(new Set()); }, [gate]);

  // 已合并 CR 触底哨兵进入视口即加载下一页（仅代码闸 + 还有更多时；提前 240px 预取）。
  useEffect(() => {
    if (gate !== 'code' || !hasMoreMerged) return;
    const el = mergedSentinelRef.current;
    const root = listScrollRef.current;
    if (!el || !root) return;
    const io = new IntersectionObserver(es => { if (es[0].isIntersecting) onLoadMoreMerged(); }, { root, rootMargin: '240px' });
    io.observe(el);
    return () => io.disconnect();
  }, [gate, hasMoreMerged, onLoadMoreMerged]);
  const allSelectableSelected = selectableIds.length > 0 && selectableIds.every(id => selected.has(id));
  const toggleSel = (id: string) => setSelected(prev => { const n = new Set(prev); n.has(id) ? n.delete(id) : n.add(id); return n; });
  const toggleAllSelectable = () => setSelected(prev => {
    const n = new Set(prev);
    if (allSelectableSelected) selectableIds.forEach(id => n.delete(id));
    else selectableIds.forEach(id => n.add(id));
    return n;
  });
  const runBatch = () => {
    if (batching || selected.size === 0) return;
    setBatching(true);
    const fn = gate === 'issue' ? onBatchApprove : onBatchApproveCrs;
    Promise.resolve(fn([...selected]))
      .finally(() => { setBatching(false); setConfirmBatch(false); setSelected(new Set()); });
  };

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
    <>
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

      {/* 顶部固定搜索框：置于滚动容器外，不随列表滚动 */}
      {(gate === 'issue' ? pendingIssues.length > 0 : crs.length > 0) && (
        <div style={{ padding: '8px 12px 4px', flexShrink: 0 }}>
          <input value={search} onChange={e => setSearch(e.target.value)} placeholder="搜索标题 / 需求编号…"
            style={{ width: '100%', boxSizing: 'border-box', background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 8, padding: '6px 10px', color: 'var(--text)', fontSize: 'var(--text-control)', outline: 'none' }} />
        </div>
      )}

      <div className="list-body scroll" ref={listScrollRef} style={{ paddingTop: 0 }}>
        {(gate === 'issue' ? pendingIssues.length === 0 : crs.length === 0) && (
          <button className="ledger-empty-cta" onClick={onOpenLedger} title="打开全量需求总账（全屏查看所有状态需求）">
            <div className="ledger-empty-cta-icon"><Icon name="list" size={26} /></div>
            <div className="ledger-empty-cta-title">{gate === 'issue' ? '暂无待审需求' : '暂无待审代码'}</div>
            <div className="ledger-empty-cta-sub">打开「全量需求总账」<br />全屏查看并管理所有状态需求</div>
            <span className="ledger-empty-cta-go">进入总账<Icon name="chevRight" size={15} /></span>
          </button>
        )}

        {/* 搜索无匹配提示：原始列表有内容但筛选后为空 */}
        {search.trim() && (gate === 'issue' ? (pendingIssues.length > 0 && filteredIssues.length === 0) : (crs.length > 0 && filteredCrs.length === 0)) && (
          <div className="empty-compact" style={{ padding: '14px 12px', textAlign: 'center', color: 'var(--text-faint)', fontSize: 'var(--text-caption)' }}>
            无匹配「{search.trim()}」的{gate === 'issue' ? '需求' : '变更'}
          </div>
        )}

        {/* 需求审核：待需求审核（Issue，尚未生成 CR） */}
        {gate === 'issue' && filteredIssues.length > 0 && (
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '8px 12px 4px', fontSize: 'var(--text-caption)', letterSpacing: '.06em', textTransform: 'uppercase', color: 'var(--text-faint)', fontWeight: 600 }}>
            <span>{STATUS_LABEL.pending_issue_review}</span>
            <span style={{ fontFamily: 'var(--font-mono)', letterSpacing: 0, color: 'var(--text-3)' }}>{filteredIssues.length}</span>
            {/* 批量审核：全选 / 取消全选可批量通过的待审核需求 */}
            {selectablePending.length > 0 && (
              <button onClick={toggleAllSelectable} title={allSelectableSelected ? '取消全选' : '全选待审核需求'}
                style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 5, background: 'none', border: 'none', cursor: 'pointer', padding: 0, color: 'var(--text-3)', fontSize: 'var(--text-caption)', fontFamily: 'var(--font-mono)', letterSpacing: 0, textTransform: 'none' }}>
                <LedgerCheck on={allSelectableSelected} /> 全选
              </button>
            )}
          </div>
        )}
        {gate === 'issue' && filteredIssues.map(issue => {
          const canSelect = issue.status === 'pending_issue_review';
          return (
            <div key={issue.id} className={'req-item' + (sel?.kind === 'issue' && sel.id === issue.id ? ' active' : '')} onClick={() => onSelectIssue(issue.id)}>
              <div className="req-item-top">
                {canSelect && (
                  <span onClick={e => { e.stopPropagation(); toggleSel(issue.id); }} style={{ display: 'flex', flexShrink: 0, cursor: 'pointer' }} title="选择以批量通过">
                    <LedgerCheck on={selected.has(issue.id)} />
                  </span>
                )}
                <span className="req-id">{issue.id.slice(0, 8)}</span>
                <span className="chip amber" style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }}>需求审核</span>
                <span className="req-time">{fmtShort(issue.updated_at)}</span>
              </div>
              <div className="req-title" style={{ fontSize: 'var(--text-control)' }} title={issue.title}>{issue.title}</div>
            </div>
          );
        })}

        {/* 代码审核 及其它 CR 状态 */}
        {gate === 'code' && (() => {
          const sorted = sortedCrs(filteredCrs);
          const statusCounts = sorted.reduce<Record<string, number>>((acc, r) => {
            acc[r.status] = (acc[r.status] ?? 0) + 1;
            return acc;
          }, {});
          let lastStatus = '';
          return sorted.map(r => {
            const showLabel = r.status !== lastStatus;
            lastStatus = r.status;
            return (
              <React.Fragment key={r.id}>
                {showLabel && (
                  <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '8px 12px 4px', fontSize: 'var(--text-caption)', letterSpacing: '.06em', textTransform: 'uppercase', color: 'var(--text-faint)', fontWeight: 600 }}>
                    <span>{STATUS_LABEL[r.status] ?? r.status}</span>
                    <span style={{ fontFamily: 'var(--font-mono)', letterSpacing: 0, color: 'var(--text-3)' }}>{statusCounts[r.status]}</span>
                    {/* 批量代码审核：仅 pending_code_review 分组显示全选，可批量通过进入合并 */}
                    {r.status === 'pending_code_review' && selectableCrs.length > 0 && (
                      <button onClick={toggleAllSelectable} title={allSelectableSelected ? '取消全选' : '全选待审核代码'}
                        style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 5, background: 'none', border: 'none', cursor: 'pointer', padding: 0, color: 'var(--text-3)', fontSize: 'var(--text-caption)', fontFamily: 'var(--font-mono)', letterSpacing: 0, textTransform: 'none' }}>
                        <LedgerCheck on={allSelectableSelected} /> 全选
                      </button>
                    )}
                  </div>
                )}
                <div className={'req-item' + (sel?.kind === 'cr' && sel.id === r.id ? ' active' : '')} onClick={() => onSelectCr(r.id)}>
                  <div className="req-item-top">
                    {r.status === 'pending_code_review' && (
                      <span onClick={e => { e.stopPropagation(); toggleSel(r.id); }} style={{ display: 'flex', flexShrink: 0, cursor: 'pointer' }} title="选择以批量通过">
                        <LedgerCheck on={selected.has(r.id)} />
                      </span>
                    )}
                    <span className="req-id">{r.id.slice(0, 8)}</span>
                    <span className={'chip ' + (STATUS_COLOR[r.status] ?? '')} style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }}>{STATUS_LABEL[r.status] ?? r.status}</span>
                    <span className="req-time">{fmtShort(r.updated_at)}</span>
                  </div>
                  <div className="req-title" style={{ fontSize: 'var(--text-control)' }} title={issueTitles[r.issue_id] || r.issue_id.slice(0, 8)}>{issueTitles[r.issue_id] || r.issue_id.slice(0, 8)}</div>
                </div>
              </React.Fragment>
            );
          });
        })()}

        {/* 已合并 CR 触底哨兵 + 加载态：仅代码闸、且开启「显示已合并」后还有更多时出现 */}
        {gate === 'code' && (
          <>
            <div ref={mergedSentinelRef} style={{ height: 1 }} />
            {mergedLoading && <div className="empty-compact" style={{ padding: '10px 0' }}>加载已合并…</div>}
          </>
        )}
      </div>

      {/* 批量审核操作条：选中后出现，一键全部通过（需求→编码 / 代码→合并） */}
      {selected.size > 0 && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '10px 14px', borderTop: '1px solid var(--border)', background: 'var(--bg-2)' }}>
          <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>已选 {selected.size}</span>
          <span style={{ flex: 1 }} />
          <button className="btn btn-sm" disabled={batching} onClick={() => setConfirmBatch(true)}
            title={gate === 'issue' ? '批量通过选中的待审核需求（进入编码）' : '批量通过选中的待审核代码（进入合并）'}
            style={{ color: 'var(--green)' }}>
            <Icon name={batching ? 'brain' : 'check'} size={13} className={batching ? 'spin' : undefined} />
            {batching ? '通过中…' : `批量通过 (${selected.size})`}
          </button>
          <button className="btn btn-sm btn-ghost" disabled={batching} onClick={() => setSelected(new Set())}>清空</button>
        </div>
      )}
    </div>
    {confirmBatch && (
      <ConfirmModal
        msg={gate === 'issue'
          ? `确定批量通过选中的 ${selected.size} 条需求？`
          : `确定批量通过选中的 ${selected.size} 条变更请求？`}
        sub={gate === 'issue'
          ? '通过后将各自生成变更请求并进入 AI 编码，无法撤销。'
          : '通过后将各自执行测试并进入自动合并，无法撤销。'}
        okLabel="批量通过"
        onOk={runBatch}
        onCancel={() => setConfirmBatch(false)}
      />
    )}
    </>
  );
}

// ── ConflictResolver：逐 hunk 决策式三方解冲突器（方案 B）+ 外部 IDE 兜底（方案 C）──
type HunkChoice = { mode: 'ours' | 'theirs' | 'both' | 'manual'; manual?: string };
const HUNK_LABELS: Record<HunkChoice['mode'], string> = {
  ours: '采用本分支', theirs: '采用 dev', both: '两者保留', manual: '手动编辑',
};
function ConflictResolver({ crId, onAiResolve, onRefresh, showError, busy }: {
  crId: string; onAiResolve: () => void; onRefresh: () => void;
  showError: (m: string) => void; busy: boolean;
}) {
  const [detail, setDetail] = useState<ConflictDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const [activeFile, setActiveFile] = useState(0);
  const [choices, setChoices] = useState<Record<string, HunkChoice>>({});
  const [submitting, setSubmitting] = useState(false);

  const load = useCallback(() => {
    setLoading(true);
    getConflictDetail(crId)
      .then(d => { setDetail(d); setActiveFile(0); setChoices({}); })
      .catch(e => showError('读取冲突现场失败：' + String(e)))
      .finally(() => setLoading(false));
  }, [crId, showError]);
  useEffect(() => { load(); }, [load]);

  const segKey = (fi: number, si: number) => `${fi}:${si}`;
  const setChoice = (fi: number, si: number, c: HunkChoice) =>
    setChoices(prev => ({ ...prev, [segKey(fi, si)]: c }));

  const preCode: React.CSSProperties = {
    margin: 0, whiteSpace: 'pre-wrap', wordBreak: 'break-word',
    fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-2)', lineHeight: 1.5,
  };

  if (loading) return <div className="empty-compact" style={{ padding: '16px 0' }}>读取冲突现场…</div>;
  if (!detail) return null;

  let totalHunks = 0, decided = 0;
  detail.files.forEach((ff, fi) => ff.segments.forEach((s, si) => {
    if (s.kind === 'conflict') { totalHunks++; if (choices[segKey(fi, si)]) decided++; }
  }));
  const allDecided = totalHunks > 0 && decided === totalHunks;

  const assemble = (): Record<string, string> | null => {
    const out: Record<string, string> = {};
    for (let fi = 0; fi < detail.files.length; fi++) {
      const ff = detail.files[fi];
      if (ff.binary) continue;
      const parts: string[] = [];
      for (let si = 0; si < ff.segments.length; si++) {
        const s = ff.segments[si];
        if (s.kind === 'context') { parts.push(s.text ?? ''); continue; }
        const c = choices[segKey(fi, si)];
        if (!c) return null;
        if (c.mode === 'ours') parts.push(s.ours ?? '');
        else if (c.mode === 'theirs') parts.push(s.theirs ?? '');
        else if (c.mode === 'both') parts.push([s.ours ?? '', s.theirs ?? ''].filter(x => x).join('\n'));
        else parts.push(c.manual ?? '');
      }
      out[ff.path] = parts.join('\n');
    }
    return out;
  };

  const doConfirm = async () => {
    const files = assemble();
    if (!files) { showError('仍有未决策的冲突块'); return; }
    setSubmitting(true);
    try { await resolveConflictManually(crId, files); onRefresh(); }
    catch (e) { showError('提交解决失败：' + String(e)); }
    finally { setSubmitting(false); }
  };
  const doExternalOpen = async () => {
    try { await openConflictWorkspace(crId); }
    catch (e) { showError('打开工作区失败：' + String(e)); }
  };
  const doExternalDone = async () => {
    setSubmitting(true);
    try { await resolveConflictManually(crId, null); onRefresh(); }
    catch (e) { showError('提交失败：' + String(e)); }
    finally { setSubmitting(false); }
  };

  if (!detail.resolvable) {
    return (
      <div>
        <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginBottom: 10 }}>
          冲突已可自动消解（rerere / 无实际冲突）。可直接提交并回到代码审核复审。
        </div>
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <button className="btn btn-primary btn-sm" disabled={submitting} onClick={doExternalDone}><Icon name="check" size={13} />提交并复审</button>
          <button className="btn btn-sm" disabled={busy || submitting} onClick={onAiResolve}><Icon name="zap" size={13} />AI 解冲突</button>
        </div>
      </div>
    );
  }

  const f = detail.files[activeFile];
  return (
    <div>
      <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginBottom: 10 }}>
        逐处选择保留哪一侧（或两者保留 / 手动编辑）。已解决 <b style={{ color: 'var(--text-2)' }}>{decided}/{totalHunks}</b>。
      </div>
      {detail.files.length > 1 && (
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6, marginBottom: 10 }}>
          {detail.files.map((ff, i) => {
            const done = ff.binary || ff.segments.every((s, si) => s.kind !== 'conflict' || choices[segKey(i, si)]);
            return (
              <button key={i} className="btn btn-sm" onClick={() => setActiveFile(i)}
                style={i === activeFile ? { borderColor: 'var(--ember)', color: 'var(--ember-soft)' } : undefined}>
                {done && <Icon name="check" size={12} />}{ff.path.split('/').pop()}
              </button>
            );
          })}
        </div>
      )}
      <div style={{ background: 'var(--code-bg)', borderRadius: 8, padding: '10px 12px', maxHeight: 420, overflow: 'auto' }}>
        {f.binary ? (
          <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)' }}>二进制文件冲突，无法逐 hunk 解决，请用「外部编辑器打开」处理。</div>
        ) : f.segments.map((s, si) => {
          if (s.kind === 'context') {
            if (!s.text) return null;
            return <pre key={si} style={{ ...preCode, color: 'var(--text-faint)' }}>{s.text}</pre>;
          }
          const c = choices[segKey(activeFile, si)];
          return (
            <div key={si} style={{ border: '1px solid var(--border-strong)', borderRadius: 8, margin: '8px 0', overflow: 'hidden' }}>
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 1, background: 'var(--border)' }}>
                <div style={{ background: 'var(--bg-2)', padding: '6px 8px' }}>
                  <div style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '.1em', marginBottom: 4 }}>dev（传入）</div>
                  <pre style={preCode}>{s.theirs || '（空）'}</pre>
                </div>
                <div style={{ background: 'var(--bg-2)', padding: '6px 8px' }}>
                  <div style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '.1em', marginBottom: 4 }}>本分支（你的）</div>
                  <pre style={preCode}>{s.ours || '（空）'}</pre>
                </div>
              </div>
              <div style={{ display: 'flex', gap: 6, padding: '6px 8px', flexWrap: 'wrap', background: 'var(--bg-2)' }}>
                {(['ours', 'theirs', 'both', 'manual'] as const).map(mode => (
                  <button key={mode} className={'chip ' + (c?.mode === mode ? 'ember' : '')} style={{ cursor: 'pointer', fontSize: 'var(--text-micro)' }}
                    onClick={() => setChoice(activeFile, si, { mode, manual: mode === 'manual' ? (c?.manual ?? [s.ours, s.theirs].filter(x => x).join('\n')) : undefined })}>
                    {HUNK_LABELS[mode]}
                  </button>
                ))}
              </div>
              {c?.mode === 'manual' && (
                <textarea value={c.manual ?? ''} onChange={e => setChoice(activeFile, si, { mode: 'manual', manual: e.target.value })}
                  style={{ width: '100%', minHeight: 80, boxSizing: 'border-box', background: 'var(--bg-3)', border: 'none', borderTop: '1px solid var(--border)', color: 'var(--text)', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', padding: '6px 8px', resize: 'vertical' }} />
              )}
            </div>
          );
        })}
      </div>
      <div style={{ display: 'flex', gap: 8, marginTop: 12, flexWrap: 'wrap' }}>
        <button className="btn btn-primary btn-sm" disabled={!allDecided || submitting} onClick={doConfirm}><Icon name="check" size={13} />确认解决并复审</button>
        <button className="btn btn-sm" disabled={busy || submitting} onClick={onAiResolve}><Icon name="zap" size={13} />AI 解冲突</button>
        <button className="btn btn-sm" disabled={submitting} onClick={doExternalOpen}><Icon name="external" size={13} />外部编辑器打开</button>
        <button className="btn btn-sm" disabled={submitting} onClick={doExternalDone}><Icon name="check" size={13} />我已在外部解决·复审</button>
      </div>
    </div>
  );
}

// ── LedgerView：全量需求总账（玻璃墙）──────────────────────────────────────────
// 只「看 / 下钻 / 整理」：所有状态可见 + 筛选搜索；状态只读、优先级不可拖；无拖拽/改状态/指派。
const LEDGER_STATUS_LABEL: Record<string, string> = {
  triage: '待整理', pending_analysis: '分析中', analysis_failed: '分析失败',
  pending_issue_review: '需求审核', pending_execution: '待编码', executing: '编码中',
  pending_code_review: '代码审核', pending_merge: '待合并', merged: '已合并',
  reverting: '撤销中', reverted: '已撤销',
  rejected: '已拒绝', merge_failed: '合并失败', merge_conflict: '合并冲突', execution_failed: '执行失败', no_change_needed: '无需改动',
};
const LEDGER_STATUS_CHIP: Record<string, string> = {
  triage: '', pending_analysis: 'amber', analysis_failed: 'red', pending_issue_review: 'amber',
  executing: 'blue', pending_code_review: 'amber', merged: 'green', rejected: '', merge_failed: 'red',
  merge_conflict: 'amber', no_change_needed: 'blue', reverting: 'amber', reverted: 'violet',
};
// 不可拒绝的状态：运行中 / 待合并 / 已合并 / 已拒绝（与后端 reject_issues 跳过集一致）。
const REJECT_SKIP = ['executing', 'building', 'running', 'pending_execution', 'pending_merge', 'merged', 'rejected'];
const canReject = (s: string) => !REJECT_SKIP.includes(s);

function LedgerCheck({ on }: { on: boolean }) {
  return (
    <span style={{ width: 16, height: 16, borderRadius: 5, border: '1px solid var(--border-strong)', background: on ? 'var(--ember)' : 'var(--bg-3)', display: 'grid', placeItems: 'center' }}>
      {on && <Icon name="check" size={11} style={{ color: 'var(--bg)' }} />}
    </span>
  );
}

const LEDGER_PAGE = 50;   // 总账每次滚动加载的条数
// 功能审计代码闸的分批加载：活动集（非合并）天然有界，一次取够；已合并历史按页滚动加载。
const CR_ACTIVE_CAP = 500;   // 活动集（非合并 CR）单次上限——远超工作流可能的在产数
const CR_MERGED_PAGE = 50;   // 已合并 CR 每次滚动加载的条数

function LedgerView({ projectId, refreshKey, sel, onSelectIssue, onRefineTriage, onRejectIssues, showMerged, onToggleMerged, mergedCount, initialStatus }: {
  projectId: string; refreshKey: number; sel: Sel | null; onSelectIssue: (id: string) => void;
  onRefineTriage: (ids: string[]) => Promise<void> | void;
  onRejectIssues: (ids: string[]) => Promise<void> | void;
  // 与功能审计页共享的「显示已合并需求」开关（默认隐藏）。
  showMerged: boolean; onToggleMerged: () => void; mergedCount: number;
  // 流水线节点跳转预置的初始状态筛选（缺省 'all'）。
  initialStatus?: string;
}) {
  const [search, setSearch] = useState('');
  const [dq, setDq] = useState('');            // 防抖后的查询串（喂给后端）
  const [statusFilter, setStatusFilter] = useState(initialStatus ?? 'all');
  // 预置筛选变化时（如已打开的总账被再次定向到另一环节）同步应用。
  useEffect(() => { if (initialStatus) setStatusFilter(initialStatus); }, [initialStatus]);
  const [statuses, setStatuses] = useState<string[]>([]);
  const [items, setItems] = useState<Issue[]>([]);   // 已加载的页（累加）
  const [total, setTotal] = useState(0);             // 当前筛选下的总数
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(false);
  // 待确认的拒绝操作：行内单条 / 批量都先弹二次确认，避免误删（triage 碎片为硬删除不可恢复）。
  const [confirmReject, setConfirmReject] = useState<null | { ids: string[]; clear: boolean }>(null);
  // 正在整理中的碎片 id：点击「整理」即时置入，用于行/按钮显示 spinner + 「整理中…」，
  // 消除 triage Agent 调用期间「点了没反应」的感知。
  const [refiningIds, setRefiningIds] = useState<Set<string>>(new Set());
  // 单调令牌：项目/筛选/刷新变化即自增，丢弃在途的过期分页响应，并区分「重置」与「追加」。
  const reqRef = useRef(0);
  const scrollRef = useRef<HTMLDivElement>(null);
  const sentinelRef = useRef<HTMLDivElement>(null);

  // 搜索防抖：停顿 250ms 再打后端，避免逐字查询。
  useEffect(() => {
    const t = setTimeout(() => setDq(search.trim()), 250);
    return () => clearTimeout(t);
  }, [search]);

  // 状态筛选 chip：取该项目出现过的全部状态（不受当前页限制）。
  useEffect(() => {
    let alive = true;
    listIssueStatuses(projectId).then(s => { if (alive) setStatuses(s); }).catch(() => { if (alive) setStatuses([]); });
    return () => { alive = false; };
  }, [projectId, refreshKey]);

  // 「显示已合并需求」关闭时（默认）：若当前正按 merged 状态筛选，回退到「全部」，避免空列表。
  useEffect(() => {
    if (!showMerged && statusFilter === 'merged') setStatusFilter('all');
  }, [showMerged, statusFilter]);

  // 项目/筛选/刷新/合并开关变化 → 重置并加载第一页。
  useEffect(() => {
    const token = ++reqRef.current;
    setLoading(true);
    listIssuesPage(projectId, statusFilter, dq, LEDGER_PAGE, 0, !showMerged)
      .then(p => { if (reqRef.current === token) { setItems(p.items); setTotal(p.total); } })
      .catch(() => { if (reqRef.current === token) { setItems([]); setTotal(0); } })
      .finally(() => { if (reqRef.current === token) setLoading(false); });
  }, [projectId, statusFilter, dq, refreshKey, showMerged]);

  const hasMore = items.length < total;
  const loadMore = useCallback(() => {
    if (loading || items.length >= total) return;
    const token = reqRef.current;  // 与当前重置同批；期间若发生重置则丢弃本次追加
    setLoading(true);
    listIssuesPage(projectId, statusFilter, dq, LEDGER_PAGE, items.length, !showMerged)
      .then(p => { if (reqRef.current === token) { setItems(prev => [...prev, ...p.items]); setTotal(p.total); } })
      .catch(() => {})
      .finally(() => { if (reqRef.current === token) setLoading(false); });
  }, [loading, items.length, total, projectId, statusFilter, dq, showMerged]);

  // 触底哨兵进入视口即加载下一页（提前 240px 预取）。
  useEffect(() => {
    const el = sentinelRef.current;
    const root = scrollRef.current;
    if (!el || !root) return;
    const io = new IntersectionObserver(es => { if (es[0].isIntersecting) loadMore(); }, { root, rootMargin: '240px' });
    io.observe(el);
    return () => io.disconnect();
  }, [loadMore]);

  // 数据刷新后剔除已不在已加载集合中的选中项，避免对幽灵 id 批量操作。
  useEffect(() => {
    setSelected(prev => {
      const valid = new Set(items.map(i => i.id));
      const next = new Set([...prev].filter(id => valid.has(id)));
      return next.size === prev.size ? prev : next;
    });
  }, [items]);

  const selectedTriage = useMemo(() => items.filter(i => selected.has(i.id) && i.status === 'triage').map(i => i.id), [items, selected]);
  const selectedRejectable = useMemo(() => items.filter(i => selected.has(i.id) && canReject(i.status)).map(i => i.id), [items, selected]);
  // 「全选」作用于已加载的行（未加载的不在内存里，无法纳入批量操作）。
  const allLoadedSelected = items.length > 0 && items.every(i => selected.has(i.id));

  const toggle = (id: string) => setSelected(prev => { const n = new Set(prev); n.has(id) ? n.delete(id) : n.add(id); return n; });
  const toggleAll = () => setSelected(prev => {
    const n = new Set(prev);
    if (allLoadedSelected) items.forEach(i => n.delete(i.id));
    else items.forEach(i => n.add(i.id));
    return n;
  });
  const run = (fn: () => Promise<void> | void, clear: boolean) => {
    if (busy) return;
    setBusy(true);
    Promise.resolve(fn()).finally(() => { setBusy(false); if (clear) setSelected(new Set()); });
  };
  // 整理可并发：每条碎片独立跑 triage Agent，互不阻塞（后端按 id 处理且带 status='triage'
  // 守卫，并发安全）。仅把本批未在途的 id 并入 refiningIds 标记 spinner，跑完只摘除自己这批，
  // 因此点一条整理时仍可点另一条同时跑。不占用全局 busy（busy 只留给拒绝等串行操作）。
  const runRefine = (ids: string[], clear: boolean) => {
    const fresh = ids.filter(id => !refiningIds.has(id));
    if (!fresh.length) return;
    setRefiningIds(prev => new Set([...prev, ...fresh]));
    Promise.resolve(onRefineTriage(fresh)).finally(() => {
      setRefiningIds(prev => { const n = new Set(prev); fresh.forEach(id => n.delete(id)); return n; });
      if (clear) setSelected(new Set());
    });
  };

  // 拒绝确认里区分 triage（硬删除）与其余（软归档），让用户清楚不可恢复的部分。
  const rejTriageCount = confirmReject ? confirmReject.ids.filter(id => items.find(i => i.id === id)?.status === 'triage').length : 0;

  return (
    <>
    <div style={{ display: 'flex', flexDirection: 'column', minHeight: 0, flex: 1 }}>
      <div className="list-body scroll" ref={scrollRef} style={{ paddingTop: 0, flex: 1 }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6, padding: '8px 12px' }}>
          <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
            <input value={search} onChange={e => setSearch(e.target.value)} placeholder="搜索标题 / 编号…"
              style={{ flex: 1, minWidth: 0, boxSizing: 'border-box', background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 8, padding: '6px 10px', color: 'var(--text)', fontSize: 'var(--text-control)', outline: 'none' }} />
            {/* 「显示已合并需求」开关：与功能审计页共享同一状态，默认隐藏。 */}
            <button className={'icon-btn' + (showMerged ? ' on' : '')} style={{ flexShrink: 0 }}
              onClick={onToggleMerged}
              title={showMerged ? `隐藏已合并需求（${mergedCount}）` : `显示已合并需求（${mergedCount}）`}>
              <Icon name={showMerged ? 'eye' : 'eye-off'} size={16} />
            </button>
          </div>
          <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
            {['all', ...statuses.filter(s => showMerged || s !== 'merged')].map(s => (
              <button key={s} onClick={() => setStatusFilter(s)}
                className={'filter-chip' + (statusFilter === s ? ' on' : '')}
                style={{ fontSize: 'var(--text-micro)', padding: '2px 8px' }}>
                {s === 'all' ? '全部' : (LEDGER_STATUS_LABEL[s] ?? s)}
              </button>
            ))}
          </div>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '0 14px 6px' }}>
          <button onClick={toggleAll} disabled={!items.length} title="全选已加载的需求"
            style={{ display: 'flex', alignItems: 'center', gap: 10, background: 'none', border: 'none', cursor: items.length ? 'pointer' : 'default', padding: 0, color: 'var(--text-3)', fontSize: 'var(--text-caption)', fontFamily: 'var(--font-mono)' }}>
            <LedgerCheck on={allLoadedSelected} /> 全选
          </button>
          <span style={{ marginLeft: 'auto', fontSize: 'var(--text-caption)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)' }}>
            {total} 条{hasMore ? `（已载 ${items.length}）` : ''}{selected.size ? ` · 已选 ${selected.size}` : ''}
          </span>
        </div>
        {items.length === 0 && !loading && <div className="empty-compact">无匹配需求</div>}
        {items.map(i => (
          <div key={i.id} className={'req-item ledger-row' + (sel?.kind === 'issue' && sel.id === i.id ? ' active' : '')} onClick={() => onSelectIssue(i.id)}>
            <span onClick={e => { e.stopPropagation(); toggle(i.id); }} style={{ display: 'flex', flexShrink: 0, cursor: 'pointer' }} title="选择">
              <LedgerCheck on={selected.has(i.id)} />
            </span>
            <span className="req-id">{i.id.slice(0, 8)}</span>
            <span className="req-title" title={i.title}>{i.title}</span>
            {i.status === 'triage' && (
              <button className="btn btn-sm" style={{ padding: '2px 8px' }} disabled={refiningIds.has(i.id)}
                onClick={e => { e.stopPropagation(); runRefine([i.id], false); }}
                title={refiningIds.has(i.id) ? 'triage Agent 整理中…' : 'triage Agent 整理成正经需求'}>
                {refiningIds.has(i.id)
                  ? <><Icon name="brain" size={12} className="spin" />整理中…</>
                  : <><Icon name="inbox" size={12} />整理</>}
              </button>
            )}
            {canReject(i.status) && (
              <button className="btn btn-sm btn-ghost" style={{ padding: '2px 6px', color: 'var(--red)' }} disabled={busy || refiningIds.has(i.id)}
                onClick={e => { e.stopPropagation(); setConfirmReject({ ids: [i.id], clear: false }); }}
                title={i.status === 'triage' ? '拒绝（删除碎片）' : '拒绝（归档为已拒绝）'}>
                <Icon name="x" size={13} />
              </button>
            )}
            <span className={'chip ' + (LEDGER_STATUS_CHIP[i.status] ?? '')} style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }}>{LEDGER_STATUS_LABEL[i.status] ?? i.status}</span>
            <span className="req-time">{fmtShort(i.updated_at)}</span>
          </div>
        ))}
        {/* 触底哨兵：进入视口即拉下一页 */}
        <div ref={sentinelRef} style={{ height: 1 }} />
        {loading && <div className="empty-compact" style={{ padding: '10px 0' }}>加载中…</div>}
        {!hasMore && !loading && items.length > 0 && (
          <div style={{ textAlign: 'center', padding: '8px 0 12px', fontSize: 'var(--text-micro)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)' }}>— 已到末尾 · 共 {total} 条 —</div>
        )}
      </div>
      {selected.size > 0 && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '10px 14px', borderTop: '1px solid var(--border)', background: 'var(--bg-2)' }}>
          <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>已选 {selected.size}</span>
          <span style={{ flex: 1 }} />
          <button className="btn btn-sm" disabled={!selectedTriage.some(id => !refiningIds.has(id))}
            onClick={() => runRefine(selectedTriage, true)} title="批量整理选中的待整理碎片">
            {refiningIds.size
              ? <><Icon name="brain" size={13} className="spin" />整理中… ({refiningIds.size})</>
              : <><Icon name="inbox" size={13} />批量整理{selectedTriage.length ? ` (${selectedTriage.length})` : ''}</>}
          </button>
          <button className="btn btn-sm btn-danger" disabled={busy || !selectedRejectable.length}
            onClick={() => setConfirmReject({ ids: selectedRejectable, clear: true })} title="批量拒绝（triage 删除 / 其余归档）">
            <Icon name="x" size={13} />批量拒绝{selectedRejectable.length ? ` (${selectedRejectable.length})` : ''}
          </button>
          <button className="btn btn-sm btn-ghost" disabled={busy} onClick={() => setSelected(new Set())}>清空</button>
        </div>
      )}
    </div>
    {confirmReject && (
      <ConfirmModal
        msg={confirmReject.ids.length > 1 ? `确定拒绝选中的 ${confirmReject.ids.length} 条需求？` : '确定拒绝该需求？'}
        sub={rejTriageCount > 0
          ? (rejTriageCount === confirmReject.ids.length
              ? '待整理碎片将被彻底删除，不可恢复。'
              : `其中 ${rejTriageCount} 条待整理碎片将被彻底删除（不可恢复），其余归档为「已拒绝」。`)
          : '将归档为「已拒绝」，不再进入流水线。'}
        okLabel="拒绝"
        onOk={() => { const c = confirmReject; setConfirmReject(null); run(() => onRejectIssues(c.ids), c.clear); }}
        onCancel={() => setConfirmReject(null)}
      />
    )}
    </>
  );
}

// ── IssueReviewView (需求审核：需求审核) ─────────────────────────────────────────

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

// 正文/列表项统一继承 .report 的基准字号（16px 衬线 prose），只补颜色，避免与段落大小不一致
const liStyle: React.CSSProperties = { color: 'var(--text-2)', lineHeight: 'var(--leading-prose)' };
const monoPath: React.CSSProperties = { fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text)' };

function AnalysisSpecView({ spec }: { spec: IssueAnalysisSpec }) {
  // 精简审核视图：默认只显示「关键核心」（需求理解 / 影响文件 / 待澄清），
  // 其余区块（根因·实现计划·验收·约束·风险·执行工单）统一收进可展开的「完整分析」。
  // 注意：仅折叠展示，不删除任何生成内容——完整 spec 仍原样喂给 Claude Code 执行。
  const [fullOpen, setFullOpen] = useState(false);
  const u = spec.understanding, rc = spec.root_cause, sc = spec.scope;
  const plan = spec.implementation_plan, b = spec.claude_code_brief;
  const steps = [...plan.steps].sort((a, z) => a.order - z.order);

  const hasRoot = !!(rc && rc.hypothesis);
  const hasPlan = !!plan.approach || steps.length > 0;
  const hasAcceptance = spec.acceptance_criteria.length > 0;
  const hasConstraints = spec.constraints.must.length > 0 || spec.constraints.must_not.length > 0;
  const hasRisks = spec.risks.length > 0;
  const hasBrief = !!(b.objective || b.instructions.length > 0);
  const hasFull = hasRoot || hasPlan || hasAcceptance || hasConstraints || hasRisks || hasBrief;

  return (
    <>
      {/* ── 关键核心：默认可见 ── */}
      {((u.restated_issue || u.restated_requirement) || u.reproduction_steps.length > 0) && (
        <>
          <SpecH2 icon="search" color="var(--blue)">需求理解</SpecH2>
          {u.problem_type && <p style={{ margin: '0 0 8px' }}><span className="chip">{u.problem_type}</span></p>}
          {(u.restated_issue || u.restated_requirement) && <p style={{ whiteSpace: 'pre-line' }}>{u.restated_issue || u.restated_requirement}</p>}
          {u.current_behavior && <p style={liStyle}><b>当前行为：</b>{u.current_behavior}</p>}
          {u.expected_behavior && <p style={liStyle}><b>期望行为：</b>{u.expected_behavior}</p>}
          {u.reproduction_steps.length > 0 && (
            <ol style={{ paddingLeft: 18, margin: '6px 0', display: 'flex', flexDirection: 'column', gap: 3 }}>
              {u.reproduction_steps.map((s, i) => <li key={i} style={liStyle}>{s}</li>)}
            </ol>
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

      {/* ── 完整分析：默认折叠，一个按钮展开全部细节（不删除生成内容）── */}
      {hasFull && (
        <div style={{ marginTop: 14 }}>
          <button className="btn btn-sm" onClick={() => setFullOpen(o => !o)}>
            <Icon name={fullOpen ? 'eye-off' : 'eye'} size={13} />{fullOpen ? '收起完整分析' : '展开完整分析（根因 · 计划 · 验收 · 约束 · 风险 · 执行工单）'}
          </button>
          {fullOpen && (
            <div style={{ marginTop: 8 }}>
              {hasRoot && (
                <>
                  <SpecH2 icon="alert" color="var(--amber)">根因分析</SpecH2>
                  <p style={{ whiteSpace: 'pre-line' }}>{rc!.hypothesis}</p>
                  {rc!.suspected_locations.length > 0 && (
                    <div style={{ display: 'flex', flexDirection: 'column', gap: 6, margin: '6px 0' }}>
                      {rc!.suspected_locations.map((l, i) => (
                        <div key={i} style={liStyle}>
                          <span style={monoPath}>{l.file}{l.symbol ? ` :: ${l.symbol}` : ''}</span>
                          <span style={{ color: 'var(--text-3)' }}> — {l.reason}</span>
                        </div>
                      ))}
                    </div>
                  )}
                </>
              )}

              {hasPlan && (
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

              {hasAcceptance && (
                <>
                  <SpecH2 icon="check" color="var(--green)">验收标准</SpecH2>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
                    {spec.acceptance_criteria.map((ac, i) => (
                      <div key={i} style={liStyle}><span style={{ fontFamily: 'var(--font-mono)', color: 'var(--green)', fontSize: 'var(--text-caption)' }}>{ac.id}</span> {ac.statement}</div>
                    ))}
                  </div>
                </>
              )}

              {hasConstraints && (
                <>
                  <SpecH2 icon="shield" color="var(--blue)">约束</SpecH2>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                    {spec.constraints.must.map((m, i) => <div key={'m' + i} style={liStyle}><span style={{ color: 'var(--green)' }}>✓</span> {m}</div>)}
                    {spec.constraints.must_not.map((m, i) => <div key={'n' + i} style={liStyle}><span style={{ color: 'var(--red)' }}>✕</span> {m}</div>)}
                  </div>
                </>
              )}

              {hasRisks && (
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

              {hasBrief && (
                <>
                  <SpecH2 icon="code" color="var(--text-3)">Claude Code 执行工单</SpecH2>
                  <div className="panel" style={{ padding: '12px 14px' }}>
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
                </>
              )}
            </div>
          )}
        </div>
      )}
    </>
  );
}

// Bug 载体只读展示（复现/环境/期望/实际）——喂自主修复的高质量输入。
function BugCarrier({ issue }: { issue: Issue }) {
  const rows = [
    ['复现步骤', issue.repro_steps], ['环境', issue.environment],
    ['期望结果', issue.expected], ['实际结果', issue.actual],
  ].filter(([, v]) => v && String(v).trim());
  if (rows.length === 0) return null;
  return (
    <>
      <h2><Icon name="alert" size={18} style={{ color: 'var(--amber)' }} />Bug 载体</h2>
      {rows.map(([label, val]) => (
        <div key={label as string} style={{ marginBottom: 8 }}>
          <div style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', textTransform: 'uppercase', letterSpacing: '.08em', color: 'var(--text-faint)', marginBottom: 3 }}>{label}</div>
          <p style={{ whiteSpace: 'pre-line', margin: 0 }}>{val}</p>
        </div>
      ))}
    </>
  );
}

// AI 生成的验收标准（人审改）——code agent 的 DoD + review_2 核对依据。
function AcceptancePanel({ issue }: { issue: Issue }) {
  type Crit = { id?: string; statement: string; verify?: string | null };
  const [criteria, setCriteria] = useState<Crit[]>([]);
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState('');
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState('');

  useEffect(() => {
    try { const a = issue.acceptance_json ? JSON.parse(issue.acceptance_json) : []; setCriteria(Array.isArray(a) ? a : []); }
    catch { setCriteria([]); }
    setEditing(false); setErr('');
  }, [issue.id, issue.acceptance_json]);

  const save = async () => {
    setSaving(true); setErr('');
    try {
      const parsed = JSON.parse(text);
      if (!Array.isArray(parsed)) throw new Error('需为 JSON 数组');
      await updateIssueAcceptance(issue.id, JSON.stringify(parsed));
      setCriteria(parsed); setEditing(false);
    } catch (e) { setErr('JSON 非法：' + String(e)); }
    finally { setSaving(false); }
  };

  if (criteria.length === 0 && !editing) return null;
  return (
    <>
      <h2 style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <Icon name="check" size={18} style={{ color: 'var(--green)' }} />验收标准
        <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', fontWeight: 400 }}>AI 生成 · 可审改</span>
        {!editing && (
          <button className="btn btn-sm btn-accent" style={{ marginLeft: 'auto' }} onClick={() => { setText(JSON.stringify(criteria, null, 2)); setEditing(true); }}>
            <Icon name="edit" size={13} />编辑
          </button>
        )}
      </h2>
      {editing ? (
        <div>
          <textarea value={text} onChange={e => setText(e.target.value)} rows={8}
            style={{ width: '100%', boxSizing: 'border-box', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 8, padding: '10px 12px', color: 'var(--text)', resize: 'vertical', outline: 'none' }} />
          {err && <div style={{ color: 'var(--red)', fontSize: 'var(--text-label)', marginTop: 6 }}>{err}</div>}
          <div style={{ display: 'flex', gap: 8, marginTop: 8 }}>
            <button className="btn btn-sm btn-primary" disabled={saving} onClick={save}><Icon name="check" size={12} />保存</button>
            <button className="btn btn-sm btn-ghost" onClick={() => setEditing(false)}>取消</button>
          </div>
        </div>
      ) : (
        <ul style={{ margin: 0, paddingLeft: 18 }}>
          {criteria.map((c, i) => (
            <li key={c.id || i} style={{ marginBottom: 6 }}>
              {c.statement || JSON.stringify(c)}
              {c.verify && <span style={{ color: 'var(--text-3)', fontSize: 'var(--text-label)' }}> — 验证：{c.verify}</span>}
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

function IssueReviewView({ issue, analysis, analysisLoading, submitting, decided, advice, setAdvice, onDecide, onRetryAnalysis, onReanalyze }: {
  issue: Issue; analysis: IssueAnalysis | null; analysisLoading: boolean;
  submitting: boolean; decided: string | null;
  advice: string; setAdvice: (v: string) => void;
  onDecide: (decision: 'approved' | 'rejected') => void;
  onRetryAnalysis: () => void;
  onReanalyze: () => void;
}) {
  const canReview = issue.status === 'pending_issue_review' && !decided;
  const analysisFailed = issue.status === 'analysis_failed';
  const spec = parseAnalysisSpec(analysis?.analysis_json);
  // Analysis concluded the requirement needs no code change (misjudgment /
  // already satisfied / pure question). Don't recommend sending it to coding —
  // demote 批准 from the primary action and surface the reason. Human still decides.
  const notRecommended = spec?.triage?.needs_changes === false;

  // 全屏阅读模式（与代码审核 / 会议室阅读模式风格一致）：衬线字体 + 报纸波点底纹
  const [fsReader, setFsReader] = useState(false);
  const reqReaderScrollRef = useRef<HTMLDivElement>(null);
  const [readerScale, setReaderScale] = useState(() => {
    const v = Number(localStorage.getItem('audit.diffScale'));
    return v >= 0.85 && v <= 2 ? v : 1.1;
  });
  const bumpScale = (delta: number) => setReaderScale(s => {
    const next = Math.min(2, Math.max(0.85, Math.round((s + delta) * 100) / 100));
    localStorage.setItem('audit.diffScale', String(next));
    return next;
  });
  useEffect(() => {
    if (!fsReader) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setFsReader(false); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [fsReader]);
  // 切换需求时退出全屏，避免残留覆盖到新选中项
  useEffect(() => { setFsReader(false); }, [issue.id]);

  // 需求描述 + 分析结果正文——内嵌视图与全屏阅读共用（全屏时不限 760，交由 CSS --measure 控制）
  const renderReportBody = () => (
    <div className="report">
      <h2><Icon name="inbox" size={18} style={{ color: 'var(--ember)' }} />需求描述</h2>
      <p style={{ whiteSpace: 'pre-line' }}>{issue.description || '（无描述）'}</p>

      <BugCarrier issue={issue} />
      <AcceptancePanel issue={issue} />

      {analysisFailed && (
        <div className="chip red" style={{ display: 'block', padding: '12px 14px', margin: '12px 0', lineHeight: 'var(--leading-normal)' }}>
          <strong>自动分析失败</strong> · 可能是 LLM 超时、限流或未配置可用模型。已保留原始错误（见下方分析摘要），可点击右上角「重新分析」重试。
        </div>
      )}

      {analysisLoading ? (
        <div className="empty-compact" style={{ padding: '20px 0' }}>加载分析…</div>
      ) : analysis ? (
        <>
          {notRecommended && (
            <div className="iter-warn" style={{ marginBottom: 12 }}>
              <Icon name="alert" size={20} />
              <div>
                <b>AI 不建议执行</b>：分析判定该需求无需改动代码{spec?.triage?.no_change_reason ? `——${spec.triage.no_change_reason}` : '（疑似误报 / 已实现 / 纯属提问）'}。建议「拒绝」关闭；如确认确有遗漏，仍可手动「批准 · 进入编码」。
              </div>
            </div>
          )}
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

    </div>
  );

  return (
    <>
      <div className="audit-top">
        <div className="audit-top-info">
          <div className="audit-top-titlerow">
            <span className="req-id" style={{ fontSize: 'var(--text-control)' }}>{issue.id.slice(0, 10)}</span>
            <CopyIdButton value={issue.id} title="复制需求编号" />
            <span className="audit-top-title" style={{ fontWeight: 700, fontSize: 'var(--text-title)' }} title={issue.title}>{issue.title}</span>
            <span className={'chip ' + (analysisFailed ? 'red' : 'amber')}>{analysisFailed ? '分析失败' : '需求审核'}</span>
          </div>
          <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 2, display: 'flex', gap: 8 }}>
            <span className={'chip ' + (SEV_COLOR[issue.category] || 'blue')} style={{ padding: '0 7px', fontSize: 'var(--text-micro)' }}>{issue.category}</span>
            <span className={'chip ' + (SEV_COLOR[issue.severity] || '')} style={{ padding: '0 7px', fontSize: 'var(--text-micro)' }}>{issue.severity}</span>
            <span>{fmtFull(issue.updated_at)}</span>
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
                  <button className={notRecommended ? 'btn' : 'btn btn-primary'} onClick={() => onDecide('approved')} disabled={submitting}
                    title={notRecommended ? 'AI 分析认为无需改动代码，不建议进入编码；如确需执行可继续' : undefined}>
                    <Icon name="check" size={15} />批准 · 进入编码
                  </button>
                </>
              : analysisFailed
                ? <button className="btn btn-primary" onClick={onRetryAnalysis} disabled={submitting}><Icon name="refresh" size={15} />重新分析</button>
                : <span className="chip" style={{ padding: '7px 14px', fontSize: 'var(--text-control)' }}>{STATUS_LABEL[issue.status] ?? issue.status}</span>}
        </div>
      </div>

      {/* 报告区：全屏按钮改为右上角悬浮，不再占整行工具栏 */}
      <div style={{ flex: 1, minHeight: 0, position: 'relative', display: 'flex', flexDirection: 'column' }}>
        <button className="icon-btn" title="全屏阅读" onClick={() => setFsReader(true)}
          style={{ position: 'absolute', top: 8, right: 14, zIndex: 5 }}>
          <Icon name="maximize" size={15} />
        </button>
        <div className="diff-viewport scroll" style={{ flex: 1 }}>
          {renderReportBody()}
        </div>
      </div>

      {/* 全屏阅读：单栏文档铺开，衬线字体 + 报纸波点底纹（对齐会议室阅读模式） */}
      {fsReader && (
        <div className="reader-overlay diff-reader req-reader" style={{ ['--rs' as string]: String(readerScale) }}>
          <div className="reader-bar" onDoubleClick={toggleMaximizeOnDoubleClick}>
            <div className="reader-bar-info">
              <Icon name="maximize" size={15} />
              <span className="reader-bar-title">{issue.title}</span>
              <span className="reader-bar-time">{analysisFailed ? '分析失败' : '需求审核'}</span>
            </div>
            <div className="reader-bar-tools">
              <button className="icon-btn" title="缩小字号" onClick={() => bumpScale(-0.1)} disabled={readerScale <= 0.85}>
                <span style={{ fontSize: 'var(--text-label)', fontWeight: 700 }}>A−</span>
              </button>
              <span className="reader-scale-val">{Math.round(readerScale * 100)}%</span>
              <button className="icon-btn" title="放大字号" onClick={() => bumpScale(0.1)} disabled={readerScale >= 2}>
                <span style={{ fontSize: 'var(--text-section)', fontWeight: 700 }}>A+</span>
              </button>
              <div className="chat-head-sep" />
              <button className="icon-btn" title="退出全屏阅读 (Esc)" onClick={() => setFsReader(false)}>
                <Icon name="x" size={18} />
              </button>
            </div>
          </div>
          <div ref={reqReaderScrollRef} className="reader-scroll scroll">
            {renderReportBody()}
            <ReaderToc scrollRef={reqReaderScrollRef} watch={(analysisLoading ? 'l' : '') + (analysis?.id ?? '') + (spec ? 's' : '')} />
          </div>
        </div>
      )}

      {/* 底部悬浮 dock：单一管理员意见输入框，支持两种操作——
          「批准 · 进入编码」时作为给编码 Agent 的实现建议随同提交；
          「重新评估」时作为补充意见让需求带其重新分析并回到需求审核。 */}
      <div className="audit-dock">
        <div className="dock-advice">
          <span className="dock-label">管理员意见（批准时给编码 Agent / 重新评估时给分析）</span>
          <div className="dock-advice-row">
            <textarea
              value={advice}
              onChange={e => setAdvice(e.target.value)}
              placeholder={(canReview || analysisFailed) ? '填写实现建议或补充意见，再选择下方操作…' : '只读状态'}
              disabled={!canReview && !analysisFailed}
            />
            <button className="btn btn-sm" onClick={onReanalyze}
              disabled={(!canReview && !analysisFailed) || submitting || !advice.trim()}
              title={(canReview || analysisFailed) ? '带此补充意见重新分析，完成后回到需求审核' : '仅「待需求审核」或「分析失败」可重新评估'}>
              <Icon name="refresh" size={14} />重新评估
            </button>
          </div>
        </div>
      </div>
    </>
  );
}

// ── AuditPage ────────────────────────────────────────────────────────────────

export default function AuditPage({ target, onTargetConsumed, openLedger, onLedgerConsumed, stageTarget, onStageConsumed }: {
  target: { projectId: string; issueId: string } | null;
  onTargetConsumed: () => void;
  /** 由通知导航请求自动打开「全量需求总账」弹窗（新需求录入）。 */
  openLedger?: boolean;
  onLedgerConsumed?: () => void;
  /** 主页完整流水线节点跳转：按项目 + 环节定位（gate 切换或总账筛选）。 */
  stageTarget?: { projectId: string; stage: string } | null;
  onStageConsumed?: () => void;
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [activeProject, setActiveProject] = useState<Project | null>(null);
  // crs 持有「活动集（非合并 CR，有界）」+「已加载的已合并历史页」。
  // 已合并 CR 随项目生命周期无限累积，故不全量加载——默认仅取首页，按需滚动追加。
  const [crs, setCrs] = useState<ChangeRequest[]>([]);
  // 当前项目的已合并 CR 总数（供「已合并」徽标计数 + 判断是否还有下一页）。
  const [mergedTotal, setMergedTotal] = useState(0);
  // 已通过分页拉取的已合并 CR 行数 = 下一页 offset。单独计数而非由 crs 推导，
  // 这样从总账下钻按需补拉的「乱序」单条 CR 不会污染分页 offset、造成跳页漏行。
  const [mergedLoaded, setMergedLoaded] = useState(0);
  const [mergedLoading, setMergedLoading] = useState(false);
  // 单调令牌：切项目/重载即自增，丢弃在途的过期 CR 分页响应。
  const crReqRef = useRef(0);
  // 默认隐藏已合并需求，开关在 audit-launch 区域控制。
  const [showMerged, setShowMerged] = useState(false);
  const [pendingIssues, setPendingIssues] = useState<Issue[]>([]);
  const [showLedger, setShowLedger] = useState(false);
  // 总账打开时的初始状态筛选：流水线节点跳到 triage/分析中/已合并等只读环节时预置。
  const [ledgerStatus, setLedgerStatus] = useState<string | undefined>(undefined);
  // 总账刷新信号：整理/拒绝后自增，触发总账重载首页（背景事件刷新不动它，避免浏览中被重置）。
  const [ledgerRefresh, setLedgerRefresh] = useState(0);
  // 通知导航请求时自动打开总账弹窗，并立即消费该意图（避免再次进入本页时重复弹出）。
  useEffect(() => {
    if (openLedger) { setShowLedger(true); onLedgerConsumed?.(); }
  }, [openLedger, onLedgerConsumed]);
  const [issueTitles, setIssueTitles] = useState<Record<string, string>>({});
  const [issuesById, setIssuesById] = useState<Record<string, Issue>>({});
  const [origReqOpen, setOrigReqOpen] = useState(false);
  const [sel, setSel] = useState<Sel | null>(null);
  // 审核闸口：'issue'=审核需求（review 1）/ 'code'=审核代码（review 2）。
  // 列表与详情都随它切换，把两步审核分开，互不干扰。
  const [gate, setGate] = useState<'issue' | 'code'>('issue');
  // 已按项目初始化过 gate 的 projectId（每项目只自动落位一次，不覆盖用户手动切换）。
  const gateInitRef = useRef<string>('');
  const [loadedProjectId, setLoadedProjectId] = useState('');
  const [issueAnalysis, setIssueAnalysis] = useState<IssueAnalysis | null>(null);
  const [analysisLoading, setAnalysisLoading] = useState(false);
  const [session, setSession] = useState<WorktreeSession | null>(null);
  const [crPreview, setCrPreview] = useState<CrPreviewStatus | null>(null);
  const [branches, setBranches] = useState<BranchInfo[]>([]);
  const [branchPreviews, setBranchPreviews] = useState<BranchPreviewStatus[]>([]);
  // 日志窗口阶段灯从这两个 ref 取实时状态：logModal.phase 只在打开时存一次，
  // 而预览状态由后台轮询持续更新，必须经 ref 才能让阶段灯跟着进程生死翻灯。
  const branchPreviewsRef = useRef<BranchPreviewStatus[]>([]);
  branchPreviewsRef.current = branchPreviews;
  const crPreviewRef = useRef<CrPreviewStatus | null>(null);
  crPreviewRef.current = crPreview;
  const [diff, setDiff] = useState('');
  const [conflict, setConflict] = useState<MergeConflictView | null>(null);
  const [conflictBusy, setConflictBusy] = useState(false);
  // 「撤销已合并需求」二次确认弹窗开关。
  const [revertConfirm, setRevertConfirm] = useState(false);
  const [revertBusy, setRevertBusy] = useState(false);
  const [grade, setGrade] = useState<CrGrade | null>(null);
  const [diffMode, setDiffMode] = useState<'unified' | 'split'>('unified');
  const [tab, setTab] = useState<'report' | 'diff'>('report');
  const [fsReader, setFsReader] = useState(false);   // 全屏阅读模式（与会议室阅读模式风格一致）
  const [diffScale, setDiffScale] = useState(() => {
    const v = Number(localStorage.getItem('audit.diffScale'));
    return v >= 0.85 && v <= 2 ? v : 1.1;
  });
  const bumpDiffScale = (delta: number) => setDiffScale(s => {
    const next = Math.min(2, Math.max(0.85, Math.round((s + delta) * 100) / 100));
    localStorage.setItem('audit.diffScale', String(next));
    return next;
  });
  const [advice, setAdvice] = useState('');
  const [commitMsg, setCommitMsg] = useState('');  // 合并提交信息（人审可改，空则后端回退默认模板）
  const [customMsgOn, setCustomMsgOn] = useState(false);  // Settings「自定义合并提交信息」开关，默认关
  const [decided, setDecided] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  // 需求审核 / 代码审核 的「拒绝」均为不可逆决策，弹二次确认避免误点。
  const [confirmReject, setConfirmReject] = useState<null | 'review1' | 'review2'>(null);
  const [crLoading, setCrLoading] = useState(false);
  // 任务进度心跳：cr_id → 最近一次阶段说明，用于在编码/合并期间显示「活着」的进度。
  const [crProgress, setCrProgress] = useState<Record<string, { phase: string; note?: string }>>({});
  const [projectReviewCounts, setProjectReviewCounts] = useState<Record<string, number>>({});
  const [intakeOpen, setIntakeOpen] = useState(false);
  const [logModal, setLogModal] = useState<{ title: string; sig: string; load: () => Promise<string>; phase?: () => LogPhase } | null>(null);
  const [toast, setToast] = useState<ToastData | null>(null);
  // 统一系统内提示框：替代浏览器原生 alert()
  const showError = useCallback((msg: string) => setToast({ msg, tone: 'error' }), []);
  const showOk = useCallback((msg: string) => setToast({ msg, tone: 'success' }), []);
  const showInfo = useCallback((msg: string) => setToast({ msg, tone: 'info' }), []);

  // Column widths（左侧列表；右侧 audit-right 已移除）
  const [listWidth, setListWidth] = useState(300);

  // Advice textarea ref for auto-focus
  const adviceRef = useRef<HTMLTextAreaElement>(null);
  const crReportReaderScrollRef = useRef<HTMLDivElement>(null);
  // Monotonic token to discard stale CR-detail responses (handles same-id refetches after a revision)
  const loadReqRef = useRef(0);
  // web 预览：startCrPreview 后服务还在 starting，置位以便就绪时自动打开浏览器
  const autoOpenRef = useRef(false);
  // tauri 桌面应用：spawn 即返回但窗口需编译后才出现，置位给「启动中…」即时反馈
  const [crAppLaunching, setCrAppLaunching] = useState(false);
  const crAppLaunchingRef = useRef(false);
  crAppLaunchingRef.current = crAppLaunching;
  // 已发起 start_branch_preview 但后端尚未返回（worktree 首次检出可能耗时数秒）的分支。
  // 让阶段灯在这段「无任何输出」的空窗期也显示「启动中」而非误判为已退出。
  const startingBranchesRef = useRef<Set<string>>(new Set());
  // 分支预览阶段灯：进程在 branchPreviews 里就取其 status（running/starting），
  // 仍在检出/启动途中取「启动中」，都不在则视作已退出（死句柄已被后台 list 清理）。
  const branchPhase = useCallback((branch: string): LogPhase => {
    const p = branchPreviewsRef.current.find(x => x.branch === branch);
    if (p) return p.status === 'running' ? 'running' : 'starting';
    if (startingBranchesRef.current.has(branch)) return 'starting';
    return 'stopped';
  }, []);
  // CR「本次改动」预览阶段灯：web 看 dev server 状态；tauri 看桌面进程是否存活。
  const crPhase = useCallback((): LogPhase => {
    const p = crPreviewRef.current;
    if (!p) return 'starting';
    if (p.kind === 'tauri') return p.app_running ? 'running' : (crAppLaunchingRef.current ? 'starting' : 'stopped');
    if (p.status === 'running') return 'running';
    if (p.status === 'starting') return 'starting';
    return 'stopped';
  }, []);

  const activeCr = sel?.kind === 'cr' ? sel.id : '';
  const activeIssueId = sel?.kind === 'issue' ? sel.id : '';
  // 两闸口的待办计数，喂给页头分段控件的徽标。
  const reqPendingCount = pendingIssues.filter(i => i.status === 'pending_issue_review').length;
  const codePendingCount = crs.filter(c => c.status === 'pending_code_review').length;
  // 选中 CR 的 updated_at：修改/重新执行后会变化，用作 diff 重新拉取的信号
  const activeCrUpdatedAt = sel?.kind === 'cr' ? crs.find(c => c.id === sel.id)?.updated_at : undefined;

  const loadProjectReviewCounts = useCallback(async () => {
    const pending = await listChangeRequests(undefined, 'pending_code_review');
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
    // 不再全量加载 issues / CR：左栏只需「活动集 CR（非合并，有界）+ 待审需求（有界子集）」；
    // 已合并 CR 会随项目生命周期无限累积，只取首页、按需滚动追加；总账自带分页滚动加载。
    // 需求审核 列表同时纳入「分析失败」需求，让用户能看到失败原因并一键重新分析。
    const token = ++crReqRef.current;
    const [activePage, mergedPage, pending] = await Promise.all([
      listChangeRequestsPage(projectId, undefined, true, CR_ACTIVE_CAP, 0),
      listChangeRequestsPage(projectId, 'merged', false, CR_MERGED_PAGE, 0),
      listIssuesByStatuses(projectId, ['pending_issue_review', 'analysis_failed']),
    ]);
    if (crReqRef.current !== token) return;  // 期间已切项目/重载，丢弃本次结果
    const allCrs = [...activePage.items, ...mergedPage.items];
    setCrs(allCrs);
    setMergedTotal(mergedPage.total);
    setMergedLoaded(mergedPage.items.length);
    setMergedLoading(false);  // 重置：避免切项目时在途 loadMoreMerged 的 loading 卡住新项目
    setPendingIssues(pending);
    // 标题映射：只取列表里实际出现的需求（CR 关联 + 待审），批量取轻量标题，避免全量加载。
    const titleIds = Array.from(new Set([...allCrs.map(c => c.issue_id), ...pending.map(i => i.id)]));
    const titleRows = titleIds.length ? await listIssueTitles(titleIds) : [];
    if (crReqRef.current !== token) return;
    const titleMap: Record<string, string> = Object.fromEntries(titleRows.map(t => [t.id, t.title]));
    pending.forEach(i => { titleMap[i.id] = i.title; });  // 兜底
    setIssueTitles(prev => ({ ...prev, ...titleMap }));
    // 待审需求完整字段进 issuesById 缓存；报告页「需求原文」所需的选中 CR 原始需求按需补拉。
    setIssuesById(prev => ({ ...prev, ...Object.fromEntries(pending.map(i => [i.id, i])) }));
    setLoadedProjectId(projectId);
  }, []);

  // 已合并 CR 滚动加载下一页：以已分页拉取的条数为 offset，去重追加。
  const loadMoreMerged = useCallback(async () => {
    if (mergedLoading || !activeProject) return;
    if (mergedLoaded >= mergedTotal) return;
    const token = crReqRef.current;  // 与当前重置同批；期间若发生重置则丢弃本次追加
    setMergedLoading(true);
    try {
      const p = await listChangeRequestsPage(activeProject.id, 'merged', false, CR_MERGED_PAGE, mergedLoaded);
      if (crReqRef.current !== token) return;
      setCrs(prev => {
        const have = new Set(prev.map(c => c.id));
        return [...prev, ...p.items.filter(c => !have.has(c.id))];
      });
      setMergedTotal(p.total);
      setMergedLoaded(prev => prev + p.items.length);
      // 补这些已合并 CR 的需求标题（仅缺失的）。
      const missing = Array.from(new Set(p.items.map(c => c.issue_id))).filter(id => !(id in issueTitles));
      if (missing.length) {
        const rows = await listIssueTitles(missing);
        if (crReqRef.current !== token) return;
        setIssueTitles(prev => ({ ...prev, ...Object.fromEntries(rows.map(t => [t.id, t.title])) }));
      }
    } finally {
      if (crReqRef.current === token) setMergedLoading(false);
    }
  }, [mergedLoading, mergedLoaded, activeProject, mergedTotal, issueTitles]);

  // 从总账下钻到某需求的 CR：先查已载入集合，未命中再按需补拉单条并并入 crs，
  // 恢复「分批加载前全量在内存」时的下钻能力（如下钻到较早的已合并 CR）。
  const resolveCr = useCallback(async (issueId: string): Promise<ChangeRequest | undefined> => {
    const inMem = crs.find(c => c.issue_id === issueId);
    if (inMem) return inMem;
    try {
      const fetched = await getChangeRequestByIssue(issueId);
      if (fetched) {
        setCrs(prev => prev.some(c => c.id === fetched.id) ? prev : [...prev, fetched]);
        if (!(fetched.issue_id in issueTitles)) {
          const rows = await listIssueTitles([fetched.issue_id]);
          setIssueTitles(prev => ({ ...prev, ...Object.fromEntries(rows.map(t => [t.id, t.title])) }));
        }
        return fetched;
      }
    } catch { /* 取不到就按「无 CR」处理，交给调用方兜底 */ }
    return undefined;
  }, [crs, issueTitles]);

  // 整理待整理池条目：triage Agent 炼成正经需求并转入流水线。
  // triage Agent 炼成正经需求并转入流水线；反馈整理/丢弃/出错数。
  const refineTriageItems = useCallback(async (ids: string[]) => {
    if (!ids.length) return;
    showInfo(`正在调用 triage Agent 整理 ${ids.length} 条碎片，请稍候…`);
    try {
      const r = await refineTriage(ids);
      if (r.errors && !r.refined && !r.discarded) {
        showError(`整理失败 ${r.errors} 条（请检查 triage 角色的 LLM 配置）`);
      } else {
        showOk(`整理完成：转入流水线 ${r.refined} · 判为噪音丢弃 ${r.discarded}` + (r.errors ? ` · 失败 ${r.errors}` : ''));
      }
    } catch (e) { showError('整理失败：' + String(e)); }
    setLedgerRefresh(v => v + 1);
    if (activeProject) await loadList(activeProject.id);
  }, [activeProject, loadList, showError, showOk, showInfo]);

  // 批量拒绝：triage 碎片硬删除，其余软归档为 rejected，运行中/已合并跳过。
  const rejectIssuesItems = useCallback(async (ids: string[]) => {
    if (!ids.length) return;
    try {
      const r = await rejectIssues(ids);
      const parts = [];
      if (r.deleted) parts.push(`删除 ${r.deleted}`);
      if (r.rejected) parts.push(`归档 ${r.rejected}`);
      if (r.skipped) parts.push(`跳过 ${r.skipped}（运行中/已合并）`);
      showOk('已拒绝：' + (parts.join(' · ') || '无可操作项'));
    } catch (e) { showError('拒绝失败：' + String(e)); }
    setLedgerRefresh(v => v + 1);
    if (activeProject) await loadList(activeProject.id);
  }, [activeProject, loadList, showError, showOk]);

  useEffect(() => { if (activeProject) loadList(activeProject.id); }, [activeProject, loadList]);

  // 默认选中 / 校验当前选择仍有效（target 导航时跳过，交由 target effect 处理）。
  // 选择被约束在当前闸口内：审核需求闸只选 issue，审核代码闸只选 CR；
  // 切换闸口时本 effect 会重新落到该闸口的首项。
  useEffect(() => {
    if (target) return;
    if (loadedProjectId !== activeProject?.id) return;

    // 每项目首次进入：按「哪边有活」自动落到对应闸口（之后不再覆盖用户手动切换）。
    let g = gate;
    if (gateInitRef.current !== loadedProjectId) {
      gateInitRef.current = loadedProjectId;
      g = reqPendingCount === 0 && codePendingCount > 0 ? 'code' : 'issue';
      if (g !== gate) setGate(g);
    }

    // 默认选中只能落在左栏「可见」的 CR 上：默认隐藏已合并需求，否则会出现
    // 列表为空但 .content 仍自动展示某条已合并 CR 的错位。
    const visibleCrs = showMerged ? crs : crs.filter(c => c.status !== 'merged');
    if (g === 'issue') {
      if (sel?.kind === 'issue' && pendingIssues.some(i => i.id === sel.id)) return;
      setSel(pendingIssues.length ? { kind: 'issue', id: pendingIssues[0].id } : null);
    } else {
      if (sel?.kind === 'cr' && crs.some(c => c.id === sel.id)) return;
      setSel(visibleCrs.length ? { kind: 'cr', id: sortedCrs(visibleCrs)[0].id } : null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [crs, pendingIssues, loadedProjectId, activeProject, target, showMerged, gate, reqPendingCount, codePendingCount]);

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
    const tid = target.issueId;
    // 标记本项目 gate 已定，避免默认落位 effect 把跳转目标的闸口覆盖掉。
    gateInitRef.current = loadedProjectId;
    const pending = pendingIssues.find(i => i.id === tid);
    if (pending) {
      setGate('issue');
      setSel({ kind: 'issue', id: pending.id });
      setDecided(null); onTargetConsumed();
      return;
    }
    let cancelled = false;
    (async () => {
      // 分批加载下 CR 可能未载入，resolveCr 命中内存则直接用、否则按需补拉单条。
      const cr = await resolveCr(tid);
      if (cancelled) return;
      if (cr) {
        setGate('code');
        if (cr.status === 'merged') setShowMerged(true);  // 已合并 CR 需开启显示才会出现在左栏
        setSel({ kind: 'cr', id: cr.id });
        setDecided(null); onTargetConsumed();
      } else {
        // 非审核阶段需求（如待整理 triage / 已合并且无 CR）：确认存在后自动打开总账并选中
        const iss = await getIssue(tid).catch(() => null);
        if (cancelled) return;
        if (iss) {
          if (iss.status === 'merged') setShowMerged(true);  // 已合并需求需开启显示才在总账可见
          setSel({ kind: 'issue', id: tid }); setShowLedger(true);
        }
        setDecided(null); onTargetConsumed();
      }
    })();
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target, loadedProjectId, crs, pendingIssues]);

  // 流水线节点跳转（按项目 + 环节）：先切到目标项目。
  useEffect(() => {
    if (!stageTarget) return;
    if (!projects.length) return;
    const proj = projects.find(p => p.id === stageTarget.projectId);
    if (!proj) { onStageConsumed?.(); return; }
    if (proj.id !== activeProject?.id) setActiveProject(proj);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stageTarget, projects]);

  // 流水线节点跳转：目标项目数据就绪后，按环节定位到对应视图。
  //   需求审核 → 审核需求闸口；代码审核 / 执行中 → 审核代码闸口；
  //   待整理 / 分析中 / 已合并 → 打开总账并预置状态筛选（这些是只读/非审核态）。
  useEffect(() => {
    if (!stageTarget) return;
    if (loadedProjectId !== stageTarget.projectId) return;
    const stage = stageTarget.stage;
    // 标记本项目 gate 已定，避免默认落位 effect 覆盖跳转意图。
    gateInitRef.current = loadedProjectId;
    if (stage === 'pending_issue_review') {
      setShowLedger(false); setGate('issue');
    } else if (stage === 'pending_code_review' || stage === 'executing') {
      setShowLedger(false); setGate('code');
    } else {
      // triage / pending_analysis / merged：总账按状态筛选浏览。
      if (stage === 'merged') setShowMerged(true);
      setLedgerStatus(stage); setShowLedger(true);
    }
    onStageConsumed?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stageTarget, loadedProjectId]);

  // 总账关闭后清掉预置筛选，下次从常规入口打开时回到「全部」。
  useEffect(() => { if (!showLedger) setLedgerStatus(undefined); }, [showLedger]);

  // 读取「自定义合并提交信息」开关（Settings 门控降级面板）；关闭时审核页不显示输入框。
  useEffect(() => { getCustomMergeMessageEnabled().then(setCustomMsgOn).catch(() => {}); }, []);

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
    // 预填默认合并信息（与后端回退模板一致），人审可改后随「批准合并」提交
    setCommitMsg(`AutoForge merge: ${crId}`);
    setSession(null);   // 清掉上一份（含上一版本）报告，避免显示过期内容
    setDiff('');        // diff='' 时视图显示「加载中…」，重拉后替换
    setConflict(null);  // 清掉上一条 CR 的冲突现场
    autoOpenRef.current = false;  // 切换 CR 时取消上一条未完成的自动打开
    setCrAppLaunching(false);     // 切换 CR 时清掉上一条桌面应用「启动中」反馈
    setTimeout(() => adviceRef.current?.focus(), 120);

    // 报告页「需求原文」所需的原始需求：不再全量缓存，选中 CR 时按需补拉进 issuesById。
    const origIssueId = crs.find(c => c.id === crId)?.issue_id;
    (async () => {
      const [s, d, g, pv, origIssue] = await Promise.all([
        getWorktreeSession(crId),
        getCodeDiff(crId),
        getCrGrade(crId).catch(() => null),
        getCrPreview(crId).catch(() => null),
        origIssueId ? getIssue(origIssueId).catch(() => null) : Promise.resolve(null),
      ]);
      if (loadReqRef.current !== reqId) return;
      setSession(s);
      setCrPreview(pv);
      setDiff(d);
      setGrade(g);
      if (origIssue) setIssuesById(prev => ({ ...prev, [origIssue.id]: origIssue }));
      setCrLoading(false);
      // 合并冲突态：按需拉取冲突现场（文件列表 + 带标记 diff）供三方视图渲染。
      if (crs.find(c => c.id === crId)?.status === 'merge_conflict') {
        getMergeConflict(crId).then(c => { if (loadReqRef.current === reqId) setConflict(c); }).catch(() => {});
      }
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

  // 桌面应用（tauri）启动监听：spawn 后进程存活（编译中/运行中）即 app_running=true，
  // 据此把按钮从「启动中…」翻成「停止程序」；进程退出（如端口冲突快速失败）则复位。
  // 启动中或运行中都持续探测：运行中是为了在程序被外部关闭后让按钮自动复位。
  useEffect(() => {
    if (!crAppLaunching && !crPreview?.app_running) return;
    const crId = activeCr;
    if (!crId) return;
    const id = setInterval(() => {
      getCrPreview(crId).then(p => {
        if (!loadReqRef.current || activeCr !== crId) return;
        setCrPreview(p);
        if (p.app_running) setCrAppLaunching(false);  // 进程已起，结束「启动中」反馈
      }).catch(() => {});
    }, 2000);
    return () => clearInterval(id);
  }, [crAppLaunching, crPreview?.app_running, activeCr]);

  // 需求审核：选中 Issue 时加载其分析结果
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
    listen<{ type?: string; cr_id?: string; phase?: string; note?: string }>('AutoForge://event', e => {
      const ev = e.payload;
      // 进度心跳：即时更新（不防抖），让用户在长任务期间看到阶段流动。
      if (ev?.type === 'task_progress' && ev.cr_id) {
        setCrProgress(prev => ({ ...prev, [ev.cr_id as string]: { phase: ev.phase || '', note: ev.note } }));
        return;
      }
      debounced();
    }).then(fn => { unlisten = fn; });
    return () => { if (timer) clearTimeout(timer); unlisten?.(); };
  }, [activeProject, loadList, loadProjectReviewCounts]);

  const doReview = async (decision: 'approved' | 'revision' | 'rejected') => {
    if (!activeCr || submitting) return;
    setSubmitting(true);
    try {
      await review2(activeCr, {
        decision,
        suggestions: advice || undefined,
        // 仅在功能开启且批准合并时带上人审填写的提交信息；修改/拒绝不涉及合并
        commit_message: customMsgOn && decision === 'approved' ? (commitMsg.trim() || undefined) : undefined,
      });
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

  // 合并冲突闭环：一键重试合并（走 Phase 1 自动 merge-dev，dev 已含解则干净落地）。
  const doRetryMerge = async () => {
    if (!activeCr || conflictBusy) return;
    setConflictBusy(true);
    try {
      await retryMerge(activeCr);
      if (activeProject) await loadList(activeProject.id);
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    } catch (e) {
      showError('重试合并失败：' + String(e));
    } finally { setConflictBusy(false); }
  };

  // 合并冲突闭环：交 AI 自动解冲突（解完回代码审核 复审，不直接落 dev）。
  const doAiResolve = async () => {
    if (!activeCr || conflictBusy) return;
    setConflictBusy(true);
    try {
      await aiResolveMergeConflict(activeCr);
      if (activeProject) await loadList(activeProject.id);
    } catch (e) {
      showError('AI 解冲突启动失败：' + String(e));
    } finally { setConflictBusy(false); }
  };

  // 已合并需求闭环：撤销该需求的改动（在 dev 上 git revert 其 squash 提交）。
  const doRevert = async () => {
    if (!activeCr || revertBusy) return;
    setRevertBusy(true);
    try {
      await revertChangeRequest(activeCr);
      setRevertConfirm(false);
      if (activeProject) await loadList(activeProject.id);
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    } catch (e) {
      showError('撤销失败：' + String(e));
    } finally { setRevertBusy(false); }
  };

  // 失败需求闭环：彻底删除需求及其执行数据。
  const doDelete = async () => {
    if (!activeCr || submitting) return;
    setConfirmDelete(false);
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

  // 分析失败闭环：一键重新分析（回到分析队列，常用于超时/限流后的恢复）。
  const doRetryAnalysis = async () => {
    if (!activeIssueId || submitting) return;
    setSubmitting(true);
    try {
      await retryAnalysis(activeIssueId);
      if (activeProject) await loadList(activeProject.id);
      await loadProjectReviewCounts();
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    } catch (e) {
      showError('重新分析失败：' + String(e));
    } finally { setSubmitting(false); }
  };

  // 需求审核 补充意见重评：带管理员补充意见重新分析当前需求，完成后重回需求审核。
  const doReanalyze = async () => {
    if (!activeIssueId || submitting || !advice.trim()) return;
    setSubmitting(true);
    try {
      await reanalyzeWithFeedback(activeIssueId, advice.trim());
      setAdvice('');
      if (activeProject) await loadList(activeProject.id);
      await loadProjectReviewCounts();
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    } catch (e) {
      showError('提交补充意见失败：' + String(e));
    } finally { setSubmitting(false); }
  };

  // 需求审核：批准 → 创建 CR 进入编码；拒绝 → 归档（后端按设计返回 Err）。
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
      // 批准后自动切到「审核代码」闸口并跳到新生成的 CR
      if (decision === 'approved' && newCr) { setGate('code'); setSel({ kind: 'cr', id: newCr.id }); }
      setSubmitting(false);
    }
  };

  // 需求审核（批量）：一键通过选中的待审核需求，快速清空需求审核 队列。
  const doBatchReview1 = async (ids: string[]) => {
    if (!ids.length) return;
    try {
      const r = await review1Batch(ids);
      showOk(`批量通过：进入编码 ${r.approved}`
        + (r.skipped ? ` · 跳过 ${r.skipped}` : '')
        + (r.errors ? ` · 失败 ${r.errors}` : ''));
    } catch (e) {
      showError('批量通过失败：' + String(e));
    } finally {
      if (activeProject) await loadList(activeProject.id);
      await loadProjectReviewCounts();
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    }
  };

  // 代码审核（批量）：一键通过选中的待代码审核 变更请求，各自排队合并，快速清空代码审核 队列。
  const doBatchReview2 = async (ids: string[]) => {
    if (!ids.length) return;
    try {
      const r = await review2Batch(ids);
      showOk(`批量通过：进入合并 ${r.approved}`
        + (r.skipped ? ` · 跳过 ${r.skipped}` : '')
        + (r.errors ? ` · 失败 ${r.errors}` : ''));
    } catch (e) {
      showError('批量通过失败：' + String(e));
    } finally {
      if (activeProject) await loadList(activeProject.id);
      await loadProjectReviewCounts();
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
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
    // 立即开日志窗口 + 标记「启动中」：worktree 首次检出在 start 返回前可能耗时数秒，
    // 这段空窗期先给用户「启动中…」的即时反馈，而非点完毫无动静。
    startingBranchesRef.current.add(branch);
    setLogModal({ title: `启动日志 · ${branch}`, sig: `branch:${pid}:${branch}`, load: () => getBranchPreviewLog(pid, branch), phase: () => branchPhase(branch) });
    try {
      const st = await startBranchPreview(pid, branch);
      setBranchPreviews(prev => [...prev.filter(p => p.branch !== branch), st].sort((a, b) => a.branch.localeCompare(b.branch)));
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
    } catch (e) {
      showError('启动失败：' + String(e));
    } finally {
      startingBranchesRef.current.delete(branch);
    }
  }, [activeProject, branchPhase, showError]);

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
    setLogModal({ title: `启动日志 · ${branch}`, sig: `branch:${pid}:${branch}`, load: () => getBranchPreviewLog(pid, branch), phase: () => branchPhase(branch) });
  }, [activeProject, branchPhase]);

  // web 项目：在 worktree 启动 dev server，就绪后自动打开浏览器（starting 时交给轮询补打开）
  const doStartCrPreview = useCallback(async () => {
    if (!activeCr) return;
    const id = activeCr;
    try {
      const st = await startCrPreview(id);
      setCrPreview(st);
      // 与分支启动/桌面应用一致：起 dev server 即打开实时日志，
      // 让用户看到编译/启动进度，并在进程报错退出时由阶段灯翻红提示。
      setLogModal({ title: '预览日志 · 本次改动', sig: `cr:${id}`, load: () => getCrPreviewLog(id), phase: crPhase });
      if (st.status === 'running' && st.url) openUrl(st.url).catch(() => {});
      else if (st.status === 'starting') autoOpenRef.current = true;
    } catch (e) { showError('启动预览失败：' + String(e)); }
  }, [activeCr, crPhase, showError]);

  const doStopCrPreview = useCallback(async () => {
    if (!activeCr) return;
    try {
      await stopCrPreview(activeCr);
      setCrAppLaunching(false);
      setCrPreview(p => p ? { ...p, status: 'stopped', url: null, app_running: false } : null);
    } catch (e) { showError('停止失败：' + String(e)); }
  }, [activeCr]);

  const doLaunchCrApp = useCallback(async () => {
    if (!activeCr) return;
    const id = activeCr;
    // 即时反馈：桌面应用首次启动需编译 Rust（数十秒），后端 spawn 即返回、窗口稍后才出现。
    // 立刻置「启动中」防重复点击、弹出实时日志看编译进度、给一条提示说明等待。
    setCrAppLaunching(true);
    showInfo('正在启动桌面应用，首次编译可能需要数十秒，下方日志可跟踪进度…');
    setLogModal({ title: '启动日志 · 桌面应用', sig: `cr:${id}`, load: () => getCrPreviewLog(id), phase: crPhase });
    try {
      await launchCrApp(id);
    } catch (e) {
      setCrAppLaunching(false);
      showError('启动桌面应用失败：' + String(e));
      return;
    }
    // 无「窗口已打开」信号，维持一段「启动中」反馈窗口；真实进度以日志为准。
    setTimeout(() => setCrAppLaunching(false), 12000);
  }, [activeCr, crPhase, showError, showInfo]);

  const showCrPreviewLog = useCallback(() => {
    if (!activeCr) return;
    const id = activeCr;
    setLogModal({ title: '预览日志 · 本次改动', sig: `cr:${id}`, load: () => getCrPreviewLog(id), phase: crPhase });
  }, [activeCr, crPhase]);

  // 切换需求时退出全屏阅读，避免残留覆盖到新选中项
  useEffect(() => { setFsReader(false); }, [activeCr]);
  // 全屏阅读模式：Esc 退出
  useEffect(() => {
    if (!fsReader) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setFsReader(false); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [fsReader]);

  const cr = crs.find(c => c.id === activeCr);
  // 「本次改动」预览：worktree 存在才可启动（合并后 no_session → 隐藏预览按钮）
  const showCrPreview = !!crPreview && crPreview.kind !== 'none' && crPreview.status !== 'no_session';
  const selectedIssue = activeIssueId ? pendingIssues.find(i => i.id === activeIssueId) : undefined;
  const report = session?.report_content ? parseReport(session.report_content) : null;
  const hunks = diff ? parseDiff(diff) : [];
  const canRevise = cr?.status === 'pending_code_review' && !decided;

  // 「本次改动」预览的启动动作：web → 起 dev server 并自动开浏览器；tauri → 直接启动桌面程序
  const renderCrLaunch = () => {
    if (!crPreview || crPreview.kind === 'none') return null;
    const { kind, status, url, can_launch_app, app_running } = crPreview;
    if (status === 'no_session') return null;
    if (status === 'starting') {
      return (
        <button className="btn btn-sm" disabled>
          <span className="dot amber" style={{ marginRight: 4 }} />启动中…
        </button>
      );
    }
    if (kind === 'tauri') {
      // tauri：直接启动桌面程序（可访问完整 IPC），无需 iframe。
      // 进程已存活（编译中/运行中）→ 显示「停止程序」；启动中 → 旋转态；否则可启动。
      return (
        <>
          {app_running ? (
            <button className="btn btn-sm" onClick={doStopCrPreview} title="停止运行中的桌面应用">
              <span className="dot green" style={{ marginRight: 4 }} />停止程序
            </button>
          ) : (
            <button className="btn btn-sm" disabled={!can_launch_app || crAppLaunching} onClick={doLaunchCrApp}>
              {crAppLaunching
                ? <><span className="dot amber" style={{ marginRight: 4 }} />启动中…</>
                : <><Icon name="box" size={14} />启动 Tauri 程序</>}
            </button>
          )}
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

  // 报告正文（实现报告 / 无改动说明 / 合并冲突 / 失败原因）——内嵌视图与全屏分栏共用
  const renderReportBody = () => (
    <div className="report">
      {origReqOpen && (() => {
        const oi = issuesById[cr!.issue_id];
        if (!oi) return null;
        return (
          <div className="panel" style={{ marginBottom: 12, padding: '12px 14px' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap', marginBottom: 8 }}>
              <span style={{ fontWeight: 700, fontSize: 'var(--text-body)' }}>{oi.title}</span>
              <span className={'chip ' + (SEV_COLOR[oi.category] || 'blue')} style={{ fontSize: 'var(--text-micro)' }}>{oi.category}</span>
              <span className={'chip ' + (SEV_COLOR[oi.severity] || '')} style={{ fontSize: 'var(--text-micro)' }}>{oi.severity}</span>
              <span style={{ marginLeft: 'auto', fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{oi.source_type} · {fmtFull(oi.created_at)}</span>
            </div>
            <p style={{ margin: 0, whiteSpace: 'pre-line', fontSize: 'var(--text-control)', color: 'var(--text-2)', lineHeight: 'var(--leading-normal)' }}>{oi.description || '（无描述）'}</p>
          </div>
        );
      })()}
      {NO_CHANGE_STATUSES.includes(cr!.status) ? (
        <div style={{ background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderLeft: '3px solid var(--blue)', borderRadius: 10, padding: '14px 16px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10, color: 'var(--blue)', fontWeight: 700, fontSize: 'var(--text-body)' }}>
            <Icon name="check" size={18} />无需改动 · Agent 说明
          </div>
          <pre style={{ margin: 0, whiteSpace: 'pre-wrap', wordBreak: 'break-word', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-2)', lineHeight: 1.6 }}>
            {crLoading ? '加载中…' : (session?.report_content || 'Agent 执行完成但未产生代码改动，且未给出说明。')}
          </pre>
          <div style={{ marginTop: 12, fontSize: 'var(--text-label)', color: 'var(--text-3)' }}>
            这通常意味着需求是误报或已实现，无需进入代码改动。可「删除需求」清除该条，或在确认确有遗漏时「重新执行」。
          </div>
        </div>
      ) : cr!.status === 'merge_conflict' ? (
        <div style={{ background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderLeft: '3px solid var(--amber)', borderRadius: 10, padding: '14px 16px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10, color: 'var(--amber)', fontWeight: 700, fontSize: 'var(--text-body)' }}>
            <Icon name="alert" size={18} />{conflict && conflict.files.length > 0 ? '合并冲突 · 并入 dev 时发生代码冲突' : '合并受阻 · 并入 dev 后集成校验未通过'}
          </div>
          <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginBottom: 12 }}>
            {conflict && conflict.files.length > 0
              ? '该需求分支与 dev 上的其他改动冲突。可一键重试合并（dev 已含解时即可干净落地），或交由 AI 自动解决冲突（解完回到代码审核复审，不直接落 dev）。'
              : '该需求分支并入最新 dev 后测试未通过（集成破坏）。可一键重试合并，或交由 AI 修复后回到代码审核复审；也可在右上角「重新执行」基于最新 dev 重新实现。'}
          </div>
          {conflict && conflict.files.length > 0 ? (
            <>
              <ConflictResolver
                crId={cr!.id}
                busy={conflictBusy}
                onAiResolve={doAiResolve}
                onRefresh={() => { if (activeProject) loadList(activeProject.id); window.dispatchEvent(new Event('AutoForge:badges-refresh')); }}
                showError={showError}
              />
              <div style={{ marginTop: 10 }}>
                <button className="btn btn-sm" disabled={conflictBusy} onClick={doRetryMerge}>
                  <Icon name="refresh" size={13} />重试合并（dev 已含解时直接落地）
                </button>
              </div>
            </>
          ) : (
            <div style={{ display: 'flex', gap: 8 }}>
              <button className="btn btn-primary btn-sm" disabled={conflictBusy} onClick={doAiResolve}>
                <Icon name="zap" size={13} />AI 解冲突并合并
              </button>
              <button className="btn btn-sm" disabled={conflictBusy} onClick={doRetryMerge}>
                <Icon name="refresh" size={13} />重试合并
              </button>
            </div>
          )}
        </div>
      ) : FAILED_STATUSES.includes(cr!.status) ? (
        <div style={{ background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderLeft: '3px solid var(--red)', borderRadius: 10, padding: '14px 16px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10, color: 'var(--red)', fontWeight: 700, fontSize: 'var(--text-body)' }}>
            <Icon name="alert" size={18} />{STATUS_LABEL[cr!.status] ?? '执行失败'}原因
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
  );

  // 代码 Diff 正文——内嵌视图与全屏分栏共用
  const renderDiffBody = () => (
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
  );

  // 全屏阅读模式头部工具：字号缩放 + 退出（与会议室阅读模式一致）
  const renderFsTools = () => (
    <>
      <button className="icon-btn" title="缩小字号" onClick={() => bumpDiffScale(-0.1)} disabled={diffScale <= 0.85}>
        <span style={{ fontSize: 'var(--text-label)', fontWeight: 700 }}>A−</span>
      </button>
      <span className="reader-scale-val">{Math.round(diffScale * 100)}%</span>
      <button className="icon-btn" title="放大字号" onClick={() => bumpDiffScale(0.1)} disabled={diffScale >= 2}>
        <span style={{ fontSize: 'var(--text-section)', fontWeight: 700 }}>A+</span>
      </button>
      <div className="chat-head-sep" />
      <button className="icon-btn" title="退出全屏阅读 (Esc)" onClick={() => setFsReader(false)}>
        <Icon name="x" size={18} />
      </button>
    </>
  );

  return (
    <div className="audit-page">
      <div className="audit-top audit-head-main" style={{ height: 56 }}>
        <div className="eyebrow" style={{ fontSize: 'var(--text-heading)' }}>
          <span className="en">AUDIT</span><span className="cn">· 功能审计</span>
        </div>
        {activeProject && (
          <div className="seg" style={{ marginLeft: 6 }}>
            <button className={gate === 'issue' ? 'on' : ''} onClick={() => setGate('issue')} title="审核需求：决定要不要做（review 1）">
              审核需求
              {reqPendingCount > 0 && <span className="chip amber" style={{ marginLeft: 6, padding: '0 6px', fontSize: 'var(--text-micro)' }}>{reqPendingCount}</span>}
            </button>
            <button className={gate === 'code' ? 'on' : ''} onClick={() => setGate('code')} title="审核代码：决定做得对不对（review 2）">
              审核代码
              {codePendingCount > 0 && <span className="chip amber" style={{ marginLeft: 6, padding: '0 6px', fontSize: 'var(--text-micro)' }}>{codePendingCount}</span>}
            </button>
          </div>
        )}
        {activeProject && (
          <BranchLauncher
            branches={branches} branchPreviews={branchPreviews}
            onStart={doStartBranch} onStop={doStopBranch} onShowLog={showBranchLog}
            onOpenIntake={() => setIntakeOpen(true)} onOpenLedger={() => setShowLedger(true)}
            showMerged={showMerged} onToggleMerged={() => setShowMerged(v => !v)}
            mergedCount={mergedTotal}
          />
        )}
      </div>

      <div className="audit-workspace">
        {/* 1. 左侧列表 + 第一个拖拽分割线 */}
        <AuditList
          projects={projects} activeProject={activeProject}
          setActiveProject={p => { setActiveProject(p); setSel(null); }}
          projectReviewCounts={projectReviewCounts} crs={showMerged ? crs : crs.filter(c => c.status !== 'merged')} pendingIssues={pendingIssues} issueTitles={issueTitles} sel={sel}
          onSelectCr={id => { setSel({ kind: 'cr', id }); setDecided(null); }}
          onSelectIssue={id => { setSel({ kind: 'issue', id }); setDecided(null); }}
          onOpenLedger={() => setShowLedger(true)}
          onBatchApprove={doBatchReview1}
          onBatchApproveCrs={doBatchReview2}
          gate={gate}
          width={listWidth}
          hasMoreMerged={showMerged && mergedLoaded < mergedTotal}
          mergedLoading={mergedLoading}
          onLoadMoreMerged={loadMoreMerged}
        />
        <ResizeHandle onDrag={dx => setListWidth(w => Math.max(180, Math.min(520, w + dx)))} />

        <div className="content">
          {selectedIssue ? (
            <IssueReviewView
              issue={selectedIssue} analysis={issueAnalysis} analysisLoading={analysisLoading}
              submitting={submitting} decided={decided}
              advice={advice} setAdvice={setAdvice}
              onDecide={d => d === 'rejected' ? setConfirmReject('review1') : doReview1('approved')}
              onRetryAnalysis={doRetryAnalysis} onReanalyze={doReanalyze}
            />
          ) : cr ? (
            <>
              {/* 顶部标题栏 */}
              <div className="audit-top">
                <div className="audit-top-info">
                  <div className="audit-top-titlerow">
                    <span className="req-id" style={{ fontSize: 'var(--text-control)' }}>{cr.id.slice(0, 10)}</span>
                    <CopyIdButton value={cr.id} title="复制变更编号" />
                    <span className="audit-top-title" style={{ fontWeight: 700, fontSize: 'var(--text-title)' }} title={issueTitles[cr.issue_id] || 'Change Request'}>{issueTitles[cr.issue_id] || 'Change Request'}</span>
                    {session && <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)' }}>迭代 {session.iteration_count} 轮</span>}
                    {grade && <span className={'chip ' + (grade.tier === 'T3' ? 'red' : grade.tier === 'T2' ? 'amber' : grade.tier === 'T1' ? 'blue' : 'green')} title={grade.rationale}>风险 {grade.tier} · {grade.change_class}</span>}
                  </div>
                  <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 2, display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                    <span>{STATUS_LABEL[cr.status] ?? cr.status} · {fmtFull(cr.updated_at)}</span>
                    {(cr.status === 'executing' || cr.status === 'pending_merge') && crProgress[cr.id]?.note && (
                      <span style={{ color: 'var(--ember)', display: 'inline-flex', alignItems: 'center', gap: 6 }}>
                        <span className="dot amber" /> {crProgress[cr.id].note}
                      </span>
                    )}
                  </div>
                </div>
                <div className="audit-decide">
                  {RECOVERABLE_STATUSES.includes(cr.status)
                    ? <>
                        <span className={'chip ' + (STATUS_COLOR[cr.status] ?? 'red')} style={{ padding: '7px 14px', fontSize: 'var(--text-control)' }}>
                          <Icon name={NO_CHANGE_STATUSES.includes(cr.status) ? 'check' : 'alert'} size={14} />{STATUS_LABEL[cr.status] ?? cr.status}
                        </span>
                        <button className="btn btn-danger" onClick={() => setConfirmDelete(true)} disabled={submitting}><Icon name="trash" size={15} />删除需求</button>
                        {/* 冲突态的主操作是解决器里的「确认解决并复审」，故此处「重新执行」降级为次级，
                            保证每屏至多一个 .btn-primary（DESIGN）；其余失败态仍以重新执行为主操作。 */}
                        <button className={'btn' + (cr.status === 'merge_conflict' ? '' : ' btn-primary')} onClick={doRetry} disabled={submitting}><Icon name="refresh" size={15} />重新执行</button>
                      </>
                    : cr.status !== 'pending_code_review'
                    ? <>
                        <span className={'chip ' + (STATUS_COLOR[cr.status] ?? '')} style={{ padding: '7px 14px', fontSize: 'var(--text-control)' }}>
                          {STATUS_LABEL[cr.status] ?? cr.status}
                        </span>
                        {cr.status === 'merged' && (
                          session?.merge_commit
                            ? <button className="btn btn-danger" onClick={() => setRevertConfirm(true)} disabled={revertBusy} title="在 dev 上 git revert 该需求的合并提交">
                                <Icon name="refresh" size={15} />{revertBusy ? '撤销中…' : '撤销改动'}
                              </button>
                            : <span title="该需求合并早于撤销功能，无法一键撤销" style={{ fontSize: 'var(--text-label)', color: 'var(--text-faint)' }}>不可撤销</span>
                        )}
                      </>
                    : decided
                      ? <span className={'chip ' + (decided === 'approved' ? 'green' : decided === 'rejected' ? 'red' : 'amber')} style={{ padding: '7px 14px', fontSize: 'var(--text-control)' }}>
                          <Icon name={decided === 'approved' ? 'check' : decided === 'rejected' ? 'x' : 'refresh'} size={14} />
                          {decided === 'approved' ? '已批准 · 合并到 dev' : decided === 'rejected' ? '已拒绝' : '已退回 · 重新执行'}
                        </span>
                      : <>
                          <button className="btn btn-danger" onClick={() => setConfirmReject('review2')} disabled={submitting}><Icon name="x" size={15} />拒绝</button>
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
                    <div className="diff-tab-tools">
                      {tab === 'report' && issuesById[cr.issue_id] && (
                        <button className="btn btn-sm" onClick={() => setOrigReqOpen(o => !o)}>
                          <Icon name={origReqOpen ? 'eye-off' : 'eye'} size={13} />{origReqOpen ? '收起' : '查看'}需求原文
                        </button>
                      )}
                      {tab === 'diff' && (
                        <div className="seg">
                          <button className={diffMode === 'unified' ? 'on' : ''} onClick={() => setDiffMode('unified')}>
                            <Icon name="rows" size={13} style={{ verticalAlign: -2, marginRight: 4 }} />统一
                          </button>
                          <button className={diffMode === 'split' ? 'on' : ''} onClick={() => setDiffMode('split')}>
                            <Icon name="columns" size={13} style={{ verticalAlign: -2, marginRight: 4 }} />分栏
                          </button>
                        </div>
                      )}
                      <button className="icon-btn" title="全屏阅读（报告 + Diff 分栏）" onClick={() => setFsReader(true)}>
                        <Icon name="maximize" size={15} />
                      </button>
                    </div>
                  </div>
                  <div className="diff-viewport scroll">
                    {tab === 'report' ? renderReportBody() : renderDiffBody()}
                  </div>

                  {/* 底部悬浮 dock：左 = 本次改动预览启动；右 = 管理员建议 + 修改 */}
                  <div className="audit-dock">
                    {showCrPreview && (
                      <div className="dock-preview">
                        <span className="dock-label">本次改动预览</span>
                        <div className="dock-preview-actions">{renderCrLaunch()}</div>
                      </div>
                    )}
                    {customMsgOn && cr.status === 'pending_code_review' && !decided && (
                      <div className="dock-advice">
                        <span className="dock-label">合并提交信息</span>
                        <div className="dock-advice-row">
                          <input
                            value={commitMsg}
                            onChange={e => setCommitMsg(e.target.value)}
                            placeholder="留空则使用默认 AutoForge merge: <编号>"
                            title="批准合并时作为 merge --no-ff 的提交信息"
                          />
                        </div>
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

      {/* 全屏阅读模式：报告 + 代码 Diff 双栏并排，铺满整窗，可调字号（风格对齐会议室阅读模式） */}
      {fsReader && cr && (
        <div className="reader-overlay diff-reader" style={{ ['--rs' as string]: String(diffScale) }}>
          <div className="reader-bar" onDoubleClick={toggleMaximizeOnDoubleClick}>
            <div className="reader-bar-info">
              <Icon name="maximize" size={15} />
              <span className="reader-bar-title">{issueTitles[cr.issue_id] || issuesById[cr.issue_id]?.title || '变更详情'}</span>
              <span className="reader-bar-time">{STATUS_LABEL[cr.status] ?? cr.status}</span>
            </div>
            <div className="reader-bar-tools">
              {issuesById[cr.issue_id] && (
                <button className="btn btn-sm" onClick={() => setOrigReqOpen(o => !o)}>
                  <Icon name={origReqOpen ? 'eye-off' : 'eye'} size={13} />{origReqOpen ? '收起' : '查看'}需求原文
                </button>
              )}
              <div className="seg">
                <button className={diffMode === 'unified' ? 'on' : ''} onClick={() => setDiffMode('unified')}>
                  <Icon name="rows" size={13} style={{ verticalAlign: -2, marginRight: 4 }} />统一
                </button>
                <button className={diffMode === 'split' ? 'on' : ''} onClick={() => setDiffMode('split')}>
                  <Icon name="columns" size={13} style={{ verticalAlign: -2, marginRight: 4 }} />分栏
                </button>
              </div>
              {renderFsTools()}
            </div>
          </div>
          <div className="diff-reader-cols">
            <section className="diff-reader-col">
              <div className="diff-reader-col-head"><Icon name="file" size={13} />实现报告</div>
              <div ref={crReportReaderScrollRef} className="diff-reader-col-scroll scroll">
                {renderReportBody()}
                <ReaderToc scrollRef={crReportReaderScrollRef} watch={(cr?.id ?? '') + cr?.status + (report ? 'r' : '') + (origReqOpen ? 'o' : '')} className="reader-toc-compact" />
              </div>
            </section>
            <section className="diff-reader-col diff-reader-col-code">
              <div className="diff-reader-col-head"><Icon name="code" size={13} />代码 Diff</div>
              <div className="diff-reader-col-scroll scroll">{renderDiffBody()}</div>
            </section>
          </div>
        </div>
      )}

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

      {showLedger && (
        <div onMouseDown={() => setShowLedger(false)}
          style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 220 }}>
          <div onMouseDown={e => e.stopPropagation()}
            style={{ width: 'min(820px, calc(100vw - 64px))', maxHeight: 'min(680px, calc(100vh - 64px))', background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
            <div style={{ padding: '16px 20px 12px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
              <div className="eyebrow" style={{ fontSize: 'var(--text-section)' }}>
                <span className="cn">全量需求总账</span>
                <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginLeft: 8, fontFamily: 'var(--font-sans)', letterSpacing: 0, textTransform: 'none' }}>所有状态 · 看 / 下钻 / 整理</span>
              </div>
              <button className="icon-btn" onClick={() => setShowLedger(false)}><Icon name="x" size={18} /></button>
            </div>
            <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
              <LedgerView projectId={activeProject?.id ?? ''} refreshKey={ledgerRefresh} sel={sel}
                onSelectIssue={async id => {
                  // 下钻到该需求并对齐审核闸口：有变更请求(已进入代码阶段，含已合并)→切「审核代码」选中其 CR；
                  // 否则仍在需求阶段→切「审核需求」选中该需求；triage/分析中等无可下钻目标的状态保持总账打开。
                  // 标记 gate 已定，避免默认落位 effect 覆盖此次跳转意图。
                  // 分批加载下 CR 可能未载入，resolveCr 命中内存则直接用、否则按需补拉单条。
                  const cr = await resolveCr(id);
                  if (cr) {
                    gateInitRef.current = loadedProjectId;
                    if (cr.status === 'merged') setShowMerged(true);  // 已合并 CR 需开启显示才会出现在左栏
                    setGate('code'); setSel({ kind: 'cr', id: cr.id }); setDecided(null); setShowLedger(false);
                  } else if (pendingIssues.some(i => i.id === id)) {
                    gateInitRef.current = loadedProjectId;
                    setGate('issue'); setSel({ kind: 'issue', id }); setDecided(null); setShowLedger(false);
                  }
                }}
                onRefineTriage={refineTriageItems} onRejectIssues={rejectIssuesItems}
                showMerged={showMerged} onToggleMerged={() => setShowMerged(v => !v)}
                mergedCount={mergedTotal}
                initialStatus={ledgerStatus} />
            </div>
          </div>
        </div>
      )}

      {logModal && (
        <LiveLogModal key={logModal.sig} title={logModal.title} load={logModal.load} phase={logModal.phase} onClose={() => setLogModal(null)} />
      )}

      {confirmDelete && (
        <ConfirmModal
          msg="确定删除该需求？"
          sub="将一并清除其执行产物、变更请求与原始需求，且不可恢复。"
          okLabel="删除需求"
          onOk={doDelete}
          onCancel={() => setConfirmDelete(false)}
        />
      )}

      {revertConfirm && (
        <ConfirmModal
          msg="确定撤销该需求的改动？"
          sub="将在 dev 上 git revert 该需求的合并提交，生成一个撤销提交（不改写历史、可再次提交恢复）。若后续改动依赖了它，会撤销失败并提示人工处理。"
          okLabel="撤销改动"
          onOk={() => { setRevertConfirm(false); doRevert(); }}
          onCancel={() => setRevertConfirm(false)}
        />
      )}

      {confirmReject && (
        <ConfirmModal
          msg={confirmReject === 'review1' ? '确定拒绝该需求？' : '确定拒绝该变更？'}
          sub={confirmReject === 'review1'
            ? '需求将归档为「已拒绝」，不再进入编码阶段。'
            : '本次代码变更将被拒绝，不会合并到 dev 分支。'}
          okLabel="拒绝"
          onOk={() => { const k = confirmReject; setConfirmReject(null); k === 'review1' ? doReview1('rejected') : doReview('rejected'); }}
          onCancel={() => setConfirmReject(null)}
        />
      )}

      <Toast data={toast} onClose={() => setToast(null)} />
    </div>
  );
}
