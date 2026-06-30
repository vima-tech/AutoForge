import React, { useState, useEffect, useCallback, useRef, useMemo, useSyncExternalStore } from 'react';
import { listen } from '@tauri-apps/api/event';
import Icon from '../components/Icon';
import Toast, { type ToastData } from '../components/Toast';
import IntakePanel from '../components/IntakePanel';
import AttachmentBar from '../components/AttachmentBar';
import ChangeSummaryCard from '../components/ChangeSummaryCard';
import ReviewAssistCard from '../components/ReviewAssistCard';
import LifecyclePanel from '../components/LifecyclePanel';
import CompareCrModal from '../components/CompareCrModal';
import { ConfirmModal } from '../components/ProjectDialogs';
import { ReaderToc } from '../components/ReaderToc';
import { toggleMaximizeOnDoubleClick } from '../lib/window';
import { fmtShort, fmtFull } from '../utils/datetime';
import {
  listActiveProjects, listChangeRequests, listChangeRequestsPage, countPendingIssueReviews, getChangeRequestByIssue, getWorktreeSession, getCodeDiff, review2, review2Batch, getCrGrade,
  retryChangeRequest, deleteChangeRequest, retryAnalysis, reanalyzeWithFeedback, deferIssue, reactivateIssue,
  openUrl, getIssue, listIssuesPage, listIssueStatuses, listIssuesByStatuses, listIssueTitles, exportIssues,
  getIssueAnalysis, review1, review1Batch, parseAnalysisSpec, updateIssueAcceptance, refineTriage, rejectIssues,
  listMergeCandidates, review1Merge, getChangeRequestIssues, type MergeCandidate, type CrIssueRef,
  getCrPreview, startCrPreview, stopCrPreview, launchCrApp, buildCrMiniapp,
  listLocalBranches, startBranchPreview, listBranchPreviews, stopBranchPreview,
  startPreviewLogTail, stopPreviewLogTail,
  getMergeConflict, retryMerge, aiResolveMergeConflict, revertChangeRequest, restoreChangeRequest, getCustomMergeMessageEnabled, getDefaultMergeMessage,
  getConflictDetail, resolveConflictManually, openConflictWorkspace,
  issueSourceMeta, listCodeAgentRuns, getCodeAgentRun, getRunningCodeAgentLog,
  type CodeAgentRunMeta, type CodeAgentRunLog,
  type Project, type ChangeRequest, type WorktreeSession, type CrGrade,
  type CrPreviewStatus, type Issue, type IssueAnalysis, type IssueAnalysisSpec,
  type BranchInfo, type BranchPreviewStatus, type MergeConflictView,
  type ConflictDetail,
} from '../services';

type Sel = { kind: 'issue' | 'cr'; id: string };

// 模块级「整理中」碎片 id 存储：脱离 React 组件树而存在，使功能审计页卸载（切换页面）后
// 仍保持，重新挂载时恢复 spinner。triage 后端命令本就跑到完（与前端挂载无关），这里只是
// 让前端「整理中」标记跨页存活——切走页面整理不会中断，回来仍显示在途。
// 按项目隔离：每个项目持有独立的在途集合，切换项目时各看各的，绝不串台（项目1整理中不影响项目2）。
// 用 useSyncExternalStore 订阅；getSnapshot 仅在该项目集合增删时换新引用，未变时引用稳定。
const REFINING_EMPTY: ReadonlySet<string> = new Set();
const refiningStore = (() => {
  const byProject = new Map<string, Set<string>>();
  const subs = new Set<() => void>();
  const emit = () => subs.forEach(fn => fn());
  return {
    // 缺省项目（或集合为空）统一返回同一冻结空集，保证 getSnapshot 引用稳定。
    get: (projectId: string): Set<string> => byProject.get(projectId) ?? (REFINING_EMPTY as Set<string>),
    add(projectId: string, more: string[]) {
      if (!projectId || !more.length) return;
      const cur = byProject.get(projectId) ?? new Set<string>();
      byProject.set(projectId, new Set([...cur, ...more]));
      emit();
    },
    remove(projectId: string, less: string[]) {
      if (!projectId || !less.length) return;
      const cur = byProject.get(projectId);
      if (!cur) return;
      const n = new Set(cur);
      less.forEach(id => n.delete(id));
      if (n.size) byProject.set(projectId, n); else byProject.delete(projectId);
      emit();
    },
    subscribe(fn: () => void) { subs.add(fn); return () => { subs.delete(fn); }; },
  };
})();

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
  pending_analysis: '分析中',
  pending_issue_review: '待需求审核',
  pending_execution: '待执行',
  executing: 'AI 执行中',
  pending_code_review: '待代码审核',
  pending_merge: '待合并',
  merge_testing: '合并中',
  merge_ready: '待落地',
  execution_failed: '执行失败',
  merge_failed: '合并失败',
  merge_conflict: '合并冲突',
  no_change_needed: '无需改动',
  merged: '已合并',
  reverting: '撤销中',
  reverted: '已撤销',
  rejected: '已拒绝',
  deferred: '暂不处置',
};
const STATUS_COLOR: Record<string, string> = {
  analysis_failed: 'red',
  pending_analysis: 'blue',
  pending_issue_review: 'amber',
  pending_execution: 'amber',
  executing: 'blue',
  pending_code_review: 'ember',
  pending_merge: 'blue',
  merge_testing: 'blue',
  merge_ready: 'amber',
  execution_failed: 'red',
  merge_failed: 'red',
  merge_conflict: 'amber',
  no_change_needed: 'blue',
  merged: 'green',
  reverting: 'amber',
  reverted: 'violet',
  rejected: 'red',
  deferred: 'violet',
};
// Failed states float to the top so abnormal requirements are easy to find and resolve.
const STATUS_ORDER = ['execution_failed', 'merge_failed', 'merge_conflict', 'pending_code_review', 'executing', 'pending_execution', 'pending_merge', 'merge_testing', 'merge_ready', 'pending_issue_review', 'no_change_needed', 'merged', 'rejected'];
// 需求审核闸口的状态排序：失败需求置顶（需重新分析），待审需求随后，分析中（进行态）垫底。
const ISSUE_STATUS_ORDER = ['analysis_failed', 'pending_issue_review', 'pending_analysis'];

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

function sortedCrs(crs: ChangeRequest[], sortAsc = false) {
  return [...crs].sort((a, b) => {
    const ai = STATUS_ORDER.indexOf(a.status);
    const bi = STATUS_ORDER.indexOf(b.status);
    if (ai !== bi) return (ai === -1 ? 99 : ai) - (bi === -1 ? 99 : bi);
    // 统一用创建时间（与行内显示一致），方向由 sortAsc 控制；状态分组仍优先。
    return sortAsc ? a.created_at.localeCompare(b.created_at) : b.created_at.localeCompare(a.created_at);
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

// 单行日志：React.memo 让流式追加时仅渲染新增的尾行——前缀行 props 按值相等会被跳过，
// 避免每来一段增量就把上千行全量重新协调。配合 .log-line 的 content-visibility，
// 视口外的行连布局/绘制都省掉，长日志也能秒开、滚动不卡。
const LogLine = React.memo(function LogLine({ n, text, tone }: { n: number; text: string; tone: string }) {
  return (
    <div className={'log-line' + (tone ? ' ' + tone : '')}>
      <span className="log-gut">{n}</span>
      <span className="log-code">{text || ' '}</span>
    </div>
  );
});

// 预览进程生命周期阶段（喂给日志窗口头部状态灯，让用户分清「仍在启动」与「已退出/报错」）。
type LogPhase = 'starting' | 'running' | 'stopped' | null;
const LOG_PHASE_META: Record<'starting' | 'running' | 'stopped', { dot: string; label: string }> = {
  starting: { dot: 'amber', label: '启动中…' },
  running: { dot: 'green', label: '运行中' },
  stopped: { dot: 'red', label: '进程已退出 · 见日志末尾确认是否报错' },
};

// 实时日志窗口：事件驱动累积、按行高亮、跟随底部自动滚动（用户上滚则暂停跟随）。
// 打开时向后端 `start_preview_log_tail(sig)` 订阅，后端 tail 日志文件并通过 `preview_log`
// 事件（payload.key === sig）增量推送新增内容，前端只 append——不再每秒全文重取/重解析。
// `phase` 是父组件按 sig 实时计算的值（骑乘父组件已有的预览状态更新，无需弹窗自行轮询），
// 使头部状态灯能区分「启动中 / 运行中 / 已退出」——否则进程崩溃后日志只是停更，
// 用户无从判断是仍在编译还是已经报错退出。
function LiveLogModal({ title, sig, phase, onClose }: {
  title: string; sig: string; phase?: LogPhase; onClose: () => void;
}) {
  const [raw, setRaw] = useState('');
  const bodyRef = useRef<HTMLDivElement>(null);
  const followRef = useRef(true);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  // 仅挂载时订阅（组件按 sig key 重挂载切换目标）：订阅 tail 事件 + 启动后端 tail。
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    listen<{ type?: string; key?: string; chunk?: string }>('autoforge://event', e => {
      const ev = e.payload;
      if (ev?.type !== 'preview_log' || ev.key !== sig || !ev.chunk) return;
      // 上限保护：累积超 400K 字符则保留尾部 300K，避免超长 build 日志撑爆内存。
      setRaw(prev => { const n = prev + ev.chunk; return n.length > 400000 ? n.slice(-300000) : n; });
    }).then(fn => { if (cancelled) fn(); else unlisten = fn; });
    startPreviewLogTail(sig).catch(() => {});
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') closeRef.current(); };
    window.addEventListener('keydown', onKey);
    return () => {
      cancelled = true;
      unlisten?.();
      stopPreviewLogTail(sig).catch(() => {});
      window.removeEventListener('keydown', onKey);
    };
  }, [sig]);

  const lines = useMemo(() => parseLogLines(raw), [raw]);
  const livePhase = phase ?? 'starting';
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
                <LogLine key={i} n={i + 1} text={l.text} tone={l.tone} />
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
          <div className="mention-pop" style={{ right: 0, left: 'auto', top: 'calc(100% + 6px)', bottom: 'auto', minWidth: 220, marginBottom: 0, maxHeight: 360, overflowY: 'auto' }}>
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
  onSelectCr, onSelectIssue, onOpenLedger, onBatchApprove, onBatchApproveCrs, onBatchReanalyze, onBatchReject, onMerge, gate,
  width, hasMoreMerged, mergedLoading, onLoadMoreMerged }: {
  projects: Project[]; activeProject: Project | null; setActiveProject: (p: Project) => void;
  projectReviewCounts: Record<string, { issue: number; code: number }>; crs: ChangeRequest[]; pendingIssues: Issue[];
  issueTitles: Record<string, string>; sel: Sel | null;
  onSelectCr: (id: string) => void; onSelectIssue: (id: string) => void; onOpenLedger: () => void;
  // 批量需求审核：通过选中的待审核需求，返回 Promise 供调用方等待刷新。
  onBatchApprove: (ids: string[]) => Promise<void> | void;
  // 批量代码审核：通过选中的待代码审核 变更请求（各自排队合并）。
  onBatchApproveCrs: (ids: string[]) => Promise<void> | void;
  // 批量重新分析：把选中的需求（待审核 / 分析失败）重新送回分析队列。
  onBatchReanalyze: (ids: string[]) => Promise<void> | void;
  // 批量拒绝：拒绝/归档选中的需求（待审核 / 分析失败）。
  onBatchReject: (ids: string[]) => Promise<void> | void;
  // 合并需求：把多条需求合并成一个 CR + 一次执行（同文件多需求合并）。
  onMerge: (issueIds: string[], primaryId?: string) => Promise<void> | void;
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
  // 批量动作确认：需求闸有「通过进入编码 / 重新分析 / 拒绝」三种，代码闸只有「通过」。
  // null = 无待确认动作。
  const [confirmBatch, setConfirmBatch] = useState<null | 'approve' | 'reanalyze' | 'reject'>(null);
  const [batching, setBatching] = useState(false);
  // 同文件多需求合并候选（仅需求闸）：随项目/待审需求变化重算（纯规则、零 LLM）。
  const [candidates, setCandidates] = useState<MergeCandidate[]>([]);
  // 合并确认面板：待合并需求 id 集 + 可选预填候选（含共享文件/冲突提示）。null=未打开。
  const [mergePanel, setMergePanel] = useState<{ ids: string[]; candidate?: MergeCandidate } | null>(null);
  const [merging, setMerging] = useState(false);
  // 分组折叠：键为 `${gate}:${status}`，存在=该组已折叠（仅收起组内行，标题与计数仍在，可重新展开）。
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set());
  useEffect(() => {
    if (gate !== 'issue' || !activeProject) { setCandidates([]); return; }
    let alive = true;
    listMergeCandidates(activeProject.id)
      .then(c => { if (alive) setCandidates(c); })
      .catch(() => { if (alive) setCandidates([]); });
    return () => { alive = false; };
  }, [gate, activeProject?.id, pendingIssues]);
  // 需求 id → 所属强候选组（行内「可合并」chip 用）；一个需求落多组时取首个强候选。
  const issueStrongCandidate = useMemo(() => {
    const m = new Map<string, MergeCandidate>();
    for (const c of candidates) {
      if (c.strength !== 'strong') continue;
      for (const id of c.issue_ids) if (!m.has(id)) m.set(id, c);
    }
    return m;
  }, [candidates]);
  // 选区是否恰好等于某个探测出的候选组 → 合并面板可直接展示其共享文件/冲突提示。
  const candidateForSelection = (ids: string[]): MergeCandidate | undefined => {
    const key = [...ids].sort().join(',');
    return candidates.find(c => [...c.issue_ids].sort().join(',') === key);
  };
  const runMerge = (primaryId: string) => {
    if (merging || !mergePanel || mergePanel.ids.length < 2) return;
    setMerging(true);
    Promise.resolve(onMerge(mergePanel.ids, primaryId))
      .finally(() => { setMerging(false); setMergePanel(null); setSelected(new Set()); });
  };
  // 需求闸批量动作下拉：按钮过多会折行，收进单个「批量操作」下拉菜单（mention-pop 模式）。
  const [actionMenuOpen, setActionMenuOpen] = useState(false);
  const actionMenuRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!actionMenuOpen) return;
    const close = (e: PointerEvent) => {
      if (e.target instanceof Node && actionMenuRef.current?.contains(e.target)) return;
      setActionMenuOpen(false);
    };
    document.addEventListener('pointerdown', close);
    return () => document.removeEventListener('pointerdown', close);
  }, [actionMenuOpen]);
  // 选区清空 / 切闸口时收起动作菜单。
  useEffect(() => { if (selected.size === 0) setActionMenuOpen(false); }, [selected]);

  // 列表模糊过滤：按标题或需求编号（短/全 id）筛选当前闸口列表。
  const [search, setSearch] = useState('');
  // 时间排序方向：true=正序（最早在前，默认——旧需求置前避免积压），false=倒序（最新在前）。
  // 状态分组仍优先，方向只翻组内时间次序。
  const [sortAsc, setSortAsc] = useState(true);
  // 来源/状态过滤（融进搜索框，不额外占高度）：空集=不过滤；否则只显示命中所选项的需求。
  const [sourceFilter, setSourceFilter] = useState<Set<string>>(new Set());
  // 状态过滤：分析失败 / 分析中 / 待需求审核 三态混在一栏，按状态过滤便于聚焦其一。
  const [statusFilter, setStatusFilter] = useState<Set<string>>(new Set());
  const [sourceMenuOpen, setSourceMenuOpen] = useState(false);
  const sourceMenuRef = useRef<HTMLDivElement>(null);
  // 点击浮层外部关闭来源过滤菜单（与项目选择菜单一致；非模态，外点即关）。
  useEffect(() => {
    if (!sourceMenuOpen) return;
    const close = (e: PointerEvent) => {
      if (e.target instanceof Node && sourceMenuRef.current?.contains(e.target)) return;
      setSourceMenuOpen(false);
    };
    document.addEventListener('pointerdown', close);
    return () => document.removeEventListener('pointerdown', close);
  }, [sourceMenuOpen]);
  // 来源选项动态取自当前待审需求实际出现过的 source_type（不堆砌不存在的来源）。
  const availableSources = useMemo(() => {
    const set = new Set<string>();
    for (const i of pendingIssues) if (i.source_type) set.add(i.source_type);
    return Array.from(set).sort();
  }, [pendingIssues]);
  // 状态过滤选项：按 ISSUE_STATUS_ORDER 固定次序，只列当前实际出现过的状态。
  const availableStatuses = useMemo(() => {
    const set = new Set<string>();
    for (const i of pendingIssues) set.add(i.status);
    return ISSUE_STATUS_ORDER.filter(s => set.has(s));
  }, [pendingIssues]);
  const filteredIssues = useMemo(() => {
    const q = search.trim().toLowerCase();
    const filtered = pendingIssues.filter(i =>
      (sourceFilter.size === 0 || sourceFilter.has(i.source_type)) &&
      (statusFilter.size === 0 || statusFilter.has(i.status)) &&
      (!q || i.title.toLowerCase().includes(q) || i.id.toLowerCase().includes(q)));
    // 与 CR 侧 sortedCrs 一致：先按状态分组（失败需关注，置顶），同状态内按更新时间倒序，
    // 避免 pending_issue_review / analysis_failed 仅按 created_at 交错排列显得混乱。
    return filtered.sort((a, b) => {
      const ai = ISSUE_STATUS_ORDER.indexOf(a.status);
      const bi = ISSUE_STATUS_ORDER.indexOf(b.status);
      if (ai !== bi) return (ai === -1 ? 99 : ai) - (bi === -1 ? 99 : bi);
      // 统一用创建时间（updated_at 会随执行/重分析变动，不稳定）：与行内显示一致。方向由 sortAsc 控制。
      return sortAsc ? a.created_at.localeCompare(b.created_at) : b.created_at.localeCompare(a.created_at);
    });
  }, [pendingIssues, search, sourceFilter, statusFilter, sortAsc]);
  const filteredCrs = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return crs;
    return crs.filter(r =>
      (issueTitles[r.issue_id] || '').toLowerCase().includes(q) ||
      r.id.toLowerCase().includes(q) ||
      r.issue_id.toLowerCase().includes(q));
  }, [crs, issueTitles, search]);
  // 切换闸口时清空搜索与来源过滤，避免跨闸口残留筛选条件。
  useEffect(() => { setSearch(''); setSourceFilter(new Set()); setStatusFilter(new Set()); setSourceMenuOpen(false); }, [gate]);
  // 列表刷新后剔除已不存在的状态过滤项（如「分析中」分析完成后消失），避免筛选把列表卡成空。
  useEffect(() => {
    setStatusFilter(prev => {
      if (prev.size === 0) return prev;
      const avail = new Set(availableStatuses);
      const next = new Set([...prev].filter(s => avail.has(s)));
      return next.size === prev.size ? prev : next;
    });
  }, [availableStatuses]);

  // 需求闸可批量操作的对象：待需求审核 + 分析失败（后者支持重新分析 / 拒绝 / 直接通过进入编码）。
  // 批量仅作用于当前筛选可见集。
  const selectablePending = useMemo(() => filteredIssues.filter(i => i.status === 'pending_issue_review' || i.status === 'analysis_failed'), [filteredIssues]);
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
  // 切换闸口时清空批量选区与分组折叠态，避免跨闸口选区残留与底部操作条错位。
  useEffect(() => { setSelected(new Set()); setCollapsedGroups(new Set()); }, [gate]);

  // 选中需求/变更后，自动把列表滚动到选中项（若不在视口内则居中显示），
  // 省去用户在长列表里手动翻找。每个选中 id 只滚一次：列表因后台任务高频刷新时不反复抖动。
  const scrolledForRef = React.useRef<string | null>(null);
  useEffect(() => {
    if (!sel) { scrolledForRef.current = null; return; }
    if (scrolledForRef.current === sel.id) return;
    const root = listScrollRef.current;
    if (!root) return;
    const el = root.querySelector(`[data-item-id="${CSS.escape(sel.id)}"]`) as HTMLElement | null;
    if (!el) return;  // 该项尚未渲染（列表在途加载）；filteredIssues/filteredCrs 更新后会重试
    scrolledForRef.current = sel.id;
    const er = el.getBoundingClientRect();
    const rr = root.getBoundingClientRect();
    if (er.top < rr.top || er.bottom > rr.bottom) {
      // 只动列表自身的 scrollTop（不调 scrollIntoView，避免连带滚动外层内容区）。
      root.scrollTop += (er.top - rr.top) - (root.clientHeight - el.clientHeight) / 2;
    }
  }, [sel, gate, filteredIssues, filteredCrs]);

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
  // 分组折叠开关（按当前闸口 + 状态键，切闸口下方 effect 会清空）。
  const isGroupCollapsed = (status: string) => collapsedGroups.has(gate + ':' + status);
  const toggleGroupCollapse = (status: string) => setCollapsedGroups(prev => {
    const n = new Set(prev); const k = gate + ':' + status;
    n.has(k) ? n.delete(k) : n.add(k); return n;
  });
  // 「选择当前分组」：组内可批量 id 的三态（全选 / 半选 / 未选）与切换。
  const groupSelState = (ids: string[]): 'all' | 'some' | 'none' => {
    if (ids.length === 0) return 'none';
    const c = ids.reduce((acc, id) => acc + (selected.has(id) ? 1 : 0), 0);
    return c === 0 ? 'none' : c === ids.length ? 'all' : 'some';
  };
  const toggleGroupSel = (ids: string[]) => setSelected(prev => {
    const n = new Set(prev);
    const allOn = ids.length > 0 && ids.every(id => n.has(id));
    if (allOn) ids.forEach(id => n.delete(id)); else ids.forEach(id => n.add(id));
    return n;
  });
  const runBatch = () => {
    if (batching || selected.size === 0 || !confirmBatch) return;
    setBatching(true);
    const ids = [...selected];
    const fn =
      gate === 'code' ? onBatchApproveCrs
      : confirmBatch === 'reanalyze' ? onBatchReanalyze
      : confirmBatch === 'reject' ? onBatchReject
      : onBatchApprove;
    Promise.resolve(fn(ids))
      .finally(() => { setBatching(false); setConfirmBatch(null); setSelected(new Set()); });
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
                <span style={{ display: 'flex', gap: 4, flexShrink: 0 }}>
                  {(projectReviewCounts[p.id]?.issue ?? 0) > 0 && (
                    <span className="chip amber" title="待审核需求" style={{ padding: '1px 6px', fontSize: 'var(--text-micro)' }}>
                      需 {projectReviewCounts[p.id].issue}
                    </span>
                  )}
                  {(projectReviewCounts[p.id]?.code ?? 0) > 0 && (
                    <span className="chip amber" title="待审核代码" style={{ padding: '1px 6px', fontSize: 'var(--text-micro)' }}>
                      码 {projectReviewCounts[p.id].code}
                    </span>
                  )}
                </span>
              </div>
            ))}
          </div>
          )}
        </div>
      </div>

      {/* 顶部固定搜索框：置于滚动容器外，不随列表滚动。来源过滤融进框内右侧图标，不额外占高度 */}
      {(gate === 'issue' ? pendingIssues.length > 0 : crs.length > 0) && (
        <div style={{ padding: '8px 12px 4px', flexShrink: 0, display: 'flex', gap: 6, alignItems: 'center' }}>
          <div style={{ position: 'relative', flex: 1, minWidth: 0 }} ref={sourceMenuRef}>
            <input value={search} onChange={e => setSearch(e.target.value)} placeholder="搜索标题 / 需求编号…"
              style={{ width: '100%', boxSizing: 'border-box', background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 8, padding: '6px 10px', paddingRight: gate === 'issue' ? 34 : 10, color: 'var(--text)', fontSize: 'var(--text-control)', outline: 'none' }} />
            {gate === 'issue' && (availableSources.length > 0 || availableStatuses.length > 1) && (() => {
              const activeCount = sourceFilter.size + statusFilter.size;
              return (
              <>
                <button type="button" onClick={() => setSourceMenuOpen(o => !o)}
                  title={activeCount ? `过滤：已选 ${activeCount} 项` : '按状态 / 来源过滤'}
                  style={{ position: 'absolute', right: 4, top: '50%', transform: 'translateY(-50%)', width: 26, height: 26, display: 'flex', alignItems: 'center', justifyContent: 'center', background: activeCount ? 'var(--ember-tint)' : 'transparent', border: 'none', borderRadius: 6, cursor: 'pointer', color: activeCount ? 'var(--ember-soft)' : 'var(--text-3)' }}>
                  <Icon name="funnel" size={14} />
                  {activeCount > 0 && <span style={{ position: 'absolute', top: 2, right: 2, width: 6, height: 6, borderRadius: 99, background: 'var(--ember)' }} />}
                </button>
                {sourceMenuOpen && (
                  <div className="mention-pop" style={{ right: 0, left: 'auto', top: 'calc(100% + 6px)', bottom: 'auto', minWidth: 180, marginBottom: 0, zIndex: 40 }}>
                    {availableStatuses.length > 1 && (
                      <>
                        <div className="mention-pop-label" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                          <span>按状态过滤</span>
                          {statusFilter.size > 0 && (
                            <span onClick={() => setStatusFilter(new Set())} style={{ cursor: 'pointer', color: 'var(--ember-soft)', textTransform: 'none', letterSpacing: 0 }}>清除</span>
                          )}
                        </div>
                        {availableStatuses.map(st => {
                          const on = statusFilter.has(st);
                          const count = pendingIssues.filter(i => i.status === st).length;
                          return (
                            <div key={st} className={'mention-row' + (on ? ' mention-active' : '')}
                              onClick={() => setStatusFilter(prev => { const n = new Set(prev); n.has(st) ? n.delete(st) : n.add(st); return n; })}>
                              <span style={{ width: 14, flexShrink: 0, color: 'var(--ember)' }}>{on && <Icon name="check" size={13} />}</span>
                              <span className={'chip ' + (STATUS_COLOR[st] ?? '')} style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }}>{STATUS_LABEL[st] ?? st}</span>
                              <span className="rl" style={{ flexShrink: 0, marginLeft: 'auto' }}>{count}</span>
                            </div>
                          );
                        })}
                      </>
                    )}
                    {availableSources.length > 0 && (
                      <>
                        <div className="mention-pop-label" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                          <span>按来源过滤</span>
                          {sourceFilter.size > 0 && (
                            <span onClick={() => setSourceFilter(new Set())} style={{ cursor: 'pointer', color: 'var(--ember-soft)', textTransform: 'none', letterSpacing: 0 }}>清除</span>
                          )}
                        </div>
                        {availableSources.map(src => {
                          const s = issueSourceMeta(src);
                          const on = sourceFilter.has(src);
                          const count = pendingIssues.filter(i => i.source_type === src).length;
                          return (
                            <div key={src} className={'mention-row' + (on ? ' mention-active' : '')}
                              onClick={() => setSourceFilter(prev => { const n = new Set(prev); n.has(src) ? n.delete(src) : n.add(src); return n; })}>
                              <span style={{ width: 14, flexShrink: 0, color: 'var(--ember)' }}>{on && <Icon name="check" size={13} />}</span>
                              <span className="nm" style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{s.label}</span>
                              <span className="rl" style={{ flexShrink: 0 }}>{count}</span>
                            </div>
                          );
                        })}
                      </>
                    )}
                  </div>
                )}
              </>
              );
            })()}
          </div>
          <button type="button" className="icon-btn" onClick={() => setSortAsc(v => !v)}
            title={sortAsc ? '创建时间正序（最早在前）· 点击切换为倒序' : '创建时间倒序（最新在前）· 点击切换为正序'}
            style={{ flexShrink: 0 }}>
            <SortGlyph asc={sortAsc} />
          </button>
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

        {/* 搜索/状态/来源过滤无匹配提示：原始列表有内容但筛选后为空 */}
        {(search.trim() || (gate === 'issue' && (sourceFilter.size > 0 || statusFilter.size > 0))) && (gate === 'issue' ? (pendingIssues.length > 0 && filteredIssues.length === 0) : (crs.length > 0 && filteredCrs.length === 0)) && (
          <div className="empty-compact" style={{ padding: '14px 12px', textAlign: 'center', color: 'var(--text-faint)', fontSize: 'var(--text-caption)' }}>
            {search.trim() ? <>无匹配「{search.trim()}」的{gate === 'issue' ? '需求' : '变更'}</> : '当前筛选下无待审需求'}
          </div>
        )}

        {/* 需求闸口：按状态分组（分析失败 / 待审核 / 分析中），各组可折叠；
            可批量分组标题左侧带「选择当前分组」三态框，多组并存时右侧再给「全选」（跨组）。 */}
        {gate === 'issue' && filteredIssues.length > 0 && (() => {
          const statusCounts = filteredIssues.reduce<Record<string, number>>((acc, i) => {
            acc[i.status] = (acc[i.status] ?? 0) + 1;
            return acc;
          }, {});
          // 各状态分组的 id 集（「选择当前分组」按组取 id；同组状态一致故可批量性一致）。
          const idsByStatus = filteredIssues.reduce<Record<string, string[]>>((acc, i) => {
            (acc[i.status] ??= []).push(i.id);
            return acc;
          }, {});
          // 可批量分组数 ≥2 时才显示跨组「全选」，与单组的「选择当前分组」区分开。
          const selectableGroupCount = new Set(selectablePending.map(i => i.status)).size;
          let lastStatus = '';
          return filteredIssues.map(issue => {
            const canSelect = issue.status === 'pending_issue_review' || issue.status === 'analysis_failed';
            const failed = issue.status === 'analysis_failed';
            const showLabel = issue.status !== lastStatus;
            lastStatus = issue.status;
            const collapsed = isGroupCollapsed(issue.status);
            const groupIds = idsByStatus[issue.status] ?? [];
            return (
            <React.Fragment key={issue.id}>
              {showLabel && (
                <GroupHead
                  label={STATUS_LABEL[issue.status] ?? issue.status}
                  count={statusCounts[issue.status]}
                  collapsed={collapsed}
                  onToggleCollapse={() => toggleGroupCollapse(issue.status)}
                  group={canSelect ? { state: groupSelState(groupIds), onToggle: () => toggleGroupSel(groupIds) } : undefined}
                  all={canSelect && selectableGroupCount >= 2 ? { selected: allSelectableSelected, onToggle: toggleAllSelectable } : undefined}
                />
              )}
            {!collapsed && (
            <div data-item-id={issue.id} className={'req-item' + (sel?.kind === 'issue' && sel.id === issue.id ? ' active' : '')} onClick={() => onSelectIssue(issue.id)}>
              {canSelect && (
                <span className="req-check" onClick={e => { e.stopPropagation(); toggleSel(issue.id); }} title="选择以批量操作">
                  <LedgerCheck on={selected.has(issue.id)} />
                </span>
              )}
              <div className="req-item-main">
                <div className="req-item-top">
                  <span className="req-id">{issue.id.slice(0, 8)}</span>
                  <span className={'chip ' + (STATUS_COLOR[issue.status] ?? 'amber')} style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }}>{failed ? '分析失败' : issue.status === 'pending_analysis' ? '分析中' : '需求审核'}</span>
                  <span className="req-time">{fmtShort(issue.created_at)}</span>
                </div>
                <div className="req-title" style={{ fontSize: 'var(--text-control)' }} title={issue.title}>{issue.title}</div>
                {(() => {
                  const cand = issueStrongCandidate.get(issue.id);
                  if (!cand) return null;
                  return (
                    <div
                      onClick={e => { e.stopPropagation(); setSelected(new Set(cand.issue_ids)); setMergePanel({ ids: cand.issue_ids, candidate: cand }); }}
                      title={`与另外 ${cand.issue_ids.length - 1} 条需求共享 ${cand.shared_files.join('、')}，点击合并为一次变更`}
                      style={{ marginTop: 4, display: 'inline-flex', alignItems: 'center', gap: 5, cursor: 'pointer', maxWidth: '100%' }}>
                      <span className="chip ember" style={{ padding: '1px 7px', fontSize: 'var(--text-micro)', flexShrink: 0 }}>
                        <Icon name="merge" size={11} /> 可合并 {cand.issue_ids.length}
                      </span>
                      <span style={{ fontSize: 'var(--text-micro)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0 }}>{cand.shared_files[0]}</span>
                    </div>
                  );
                })()}
              </div>
            </div>
            )}
            </React.Fragment>
          );
          });
        })()}

        {/* 代码审核 及其它 CR 状态：各组可折叠；待审核代码组带「选择当前分组」三态框 */}
        {gate === 'code' && (() => {
          const sorted = sortedCrs(filteredCrs, sortAsc);
          const statusCounts = sorted.reduce<Record<string, number>>((acc, r) => {
            acc[r.status] = (acc[r.status] ?? 0) + 1;
            return acc;
          }, {});
          // 待审核代码组 id 集（「选择当前分组」用；代码闸只有此组可批量，故无需跨组「全选」）。
          const pendingIds = sorted.filter(r => r.status === 'pending_code_review').map(r => r.id);
          let lastStatus = '';
          return sorted.map(r => {
            const showLabel = r.status !== lastStatus;
            lastStatus = r.status;
            const canSelect = r.status === 'pending_code_review';
            const collapsed = isGroupCollapsed(r.status);
            return (
              <React.Fragment key={r.id}>
                {showLabel && (
                  <GroupHead
                    label={STATUS_LABEL[r.status] ?? r.status}
                    count={statusCounts[r.status]}
                    collapsed={collapsed}
                    onToggleCollapse={() => toggleGroupCollapse(r.status)}
                    group={canSelect && selectableCrs.length > 0 ? { state: groupSelState(pendingIds), onToggle: () => toggleGroupSel(pendingIds) } : undefined}
                  />
                )}
                {!collapsed && (
                <div data-item-id={r.id} className={'req-item' + (sel?.kind === 'cr' && sel.id === r.id ? ' active' : '')} onClick={() => onSelectCr(r.id)}>
                  {canSelect && (
                    <span className="req-check" onClick={e => { e.stopPropagation(); toggleSel(r.id); }} title="选择以批量通过">
                      <LedgerCheck on={selected.has(r.id)} />
                    </span>
                  )}
                  <div className="req-item-main">
                    <div className="req-item-top">
                      <span className="req-id">{r.id.slice(0, 8)}</span>
                      <span className={'chip ' + (STATUS_COLOR[r.status] ?? '')} style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }}>{STATUS_LABEL[r.status] ?? r.status}</span>
                      <span className="req-time">{fmtShort(r.created_at)}</span>
                    </div>
                    <div className="req-title" style={{ fontSize: 'var(--text-control)' }} title={issueTitles[r.issue_id] || r.issue_id.slice(0, 8)}>{issueTitles[r.issue_id] || r.issue_id.slice(0, 8)}</div>
                  </div>
                </div>
                )}
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

      {/* 批量操作条：选中后出现。代码闸只有「批量通过」单按钮；需求闸三种动作收进「批量操作」下拉，避免折行。 */}
      {selected.size > 0 && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '10px 14px', borderTop: '1px solid var(--border)', background: 'var(--bg-2)' }}>
          <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>已选 {selected.size}</span>
          <span style={{ flex: 1 }} />
          {gate === 'issue' ? (
            <div style={{ position: 'relative' }} ref={actionMenuRef}>
              <button className="btn btn-sm" disabled={batching} onClick={() => setActionMenuOpen(o => !o)}
                title="对选中的需求批量操作">
                <Icon name={batching ? 'brain' : 'layers'} size={13} className={batching ? 'spin' : undefined} />
                {batching ? '处理中…' : `批量操作 (${selected.size})`}
                <Icon name="chevDown" size={13} style={{ color: 'var(--text-3)', transition: 'transform .15s', transform: actionMenuOpen ? 'rotate(180deg)' : 'none' }} />
              </button>
              {actionMenuOpen && (
                <div className="mention-pop" style={{ right: 0, left: 'auto', bottom: 'calc(100% + 6px)', top: 'auto', minWidth: 200, marginBottom: 0 }}>
                  {selected.size >= 2 && (
                    <div className="mention-row" onClick={() => {
                      setActionMenuOpen(false);
                      const ids = [...selected];
                      setMergePanel({ ids, candidate: candidateForSelection(ids) });
                    }}
                      title="把选中的需求合并成一个变更、一次编码（适合都改同一文件的需求）">
                      <Icon name="merge" size={14} style={{ color: 'var(--ember)', flexShrink: 0 }} />
                      <div style={{ minWidth: 0, flex: 1 }}><div className="nm">合并为一次变更…</div></div>
                    </div>
                  )}
                  <div className="mention-row" onClick={() => { setActionMenuOpen(false); setConfirmBatch('approve'); }}>
                    <Icon name="check" size={14} style={{ color: 'var(--green)', flexShrink: 0 }} />
                    <div style={{ minWidth: 0, flex: 1 }}><div className="nm">通过进入编码</div></div>
                  </div>
                  <div className="mention-row" onClick={() => { setActionMenuOpen(false); setConfirmBatch('reanalyze'); }}>
                    <Icon name="refresh" size={14} style={{ color: 'var(--text-3)', flexShrink: 0 }} />
                    <div style={{ minWidth: 0, flex: 1 }}><div className="nm">重新分析</div></div>
                  </div>
                  <div className="mention-row" onClick={() => { setActionMenuOpen(false); setConfirmBatch('reject'); }}>
                    <Icon name="x" size={14} style={{ color: 'var(--red)', flexShrink: 0 }} />
                    <div style={{ minWidth: 0, flex: 1 }}><div className="nm" style={{ color: 'var(--red)' }}>拒绝</div></div>
                  </div>
                </div>
              )}
            </div>
          ) : (
            <button className="btn btn-sm" disabled={batching} onClick={() => setConfirmBatch('approve')}
              title="批量通过选中的待审核代码（进入合并）" style={{ color: 'var(--green)' }}>
              <Icon name={batching ? 'brain' : 'check'} size={13} className={batching ? 'spin' : undefined} />
              {batching ? '处理中…' : `批量通过 (${selected.size})`}
            </button>
          )}
          <button className="btn btn-sm btn-ghost" disabled={batching} onClick={() => setSelected(new Set())}>清空</button>
        </div>
      )}
    </div>
    {confirmBatch === 'approve' && (
      <ConfirmModal
        msg={gate === 'issue'
          ? `确定批量通过选中的 ${selected.size} 条需求？`
          : `确定批量通过选中的 ${selected.size} 条变更请求？`}
        sub={gate === 'issue'
          ? '通过后将各自生成变更请求并进入 AI 编码，无法撤销。其中「分析失败」的需求将跳过分析直接进入编码（仅凭原始描述）。'
          : '通过后将各自执行测试并进入自动合并，无法撤销。'}
        okLabel="批量通过"
        onOk={runBatch}
        onCancel={() => setConfirmBatch(null)}
      />
    )}
    {confirmBatch === 'reanalyze' && (
      <ConfirmModal
        msg={`确定将选中的 ${selected.size} 条需求重新分析？`}
        sub="将各自送回分析队列重新评估，完成后回到需求审核。"
        okLabel="重新分析"
        onOk={runBatch}
        onCancel={() => setConfirmBatch(null)}
      />
    )}
    {confirmBatch === 'reject' && (
      <ConfirmModal
        msg={`确定拒绝选中的 ${selected.size} 条需求？`}
        sub="拒绝后将从待办队列移除（可在总账中查看）。"
        okLabel="拒绝"
        onOk={runBatch}
        onCancel={() => setConfirmBatch(null)}
      />
    )}
    {mergePanel && (
      <MergeConfirm
        members={mergePanel.ids.map(id => ({ id, title: pendingIssues.find(i => i.id === id)?.title || id.slice(0, 8) }))}
        candidate={mergePanel.candidate}
        busy={merging}
        onConfirm={runMerge}
        onCancel={() => { if (!merging) setMergePanel(null); }}
      />
    )}
    </>
  );
}

// ── 合并确认面板：把多条需求合并成一个变更、一次编码 ────────────────────────────
// 遵守 DESIGN：遮罩 inset:var(--win-gutter) + 圆角；不点遮罩关闭，仅 ✕ / Esc；每屏 ≤1 主操作。
function MergeConfirm({ members, candidate, busy, onConfirm, onCancel }: {
  members: { id: string; title: string }[];
  candidate?: MergeCandidate;
  busy: boolean;
  onConfirm: (primaryId: string) => void;
  onCancel: () => void;
}) {
  // 主需求默认取第一条（驱动 CR 标题 / Innate 召回）。
  const [primary, setPrimary] = useState(members[0]?.id ?? '');
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape' && !busy) onCancel(); };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [busy, onCancel]);
  return (
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 230 }}>
      <div style={{ background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 14, padding: '20px 22px', width: 480, maxWidth: '90vw', maxHeight: '82vh', overflow: 'auto', boxShadow: 'var(--shadow-lg)' }} onClick={e => e.stopPropagation()}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 14 }}>
          <Icon name="merge" size={16} style={{ color: 'var(--ember)' }} />
          <h3 style={{ margin: 0, fontSize: 'var(--text-section)', fontFamily: 'var(--font-display)' }}>合并 {members.length} 个需求为一次变更</h3>
          <span style={{ flex: 1 }} />
          <button className="icon-btn" onClick={onCancel} disabled={busy} title="关闭（Esc）"><Icon name="x" size={16} /></button>
        </div>

        {/* 共享文件（探测出的候选才有；手动合并则提示无预查信息） */}
        {candidate && candidate.shared_files.length > 0 ? (
          <div style={{ marginBottom: 12 }}>
            <div className="field-label" style={{ marginBottom: 6 }}>共享文件</div>
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
              {candidate.shared_files.map(f => (
                <span key={f} className="chip ember" style={{ fontSize: 'var(--text-micro)', padding: '2px 8px' }}>{f}</span>
              ))}
            </div>
            <div style={{ marginTop: 6, fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>
              合并后预计改动 {candidate.total_files} 个文件
            </div>
          </div>
        ) : (
          <div style={{ marginBottom: 12, fontSize: 'var(--text-caption)', color: 'var(--text-3)', lineHeight: 'var(--leading-relaxed)' }}>
            手动合并（非系统建议组）：将把这些需求作为同一变更一次性实现，请确认它们确属相关改动。
          </div>
        )}

        {/* 冲突提示 */}
        {candidate?.conflict_hint && (
          <div style={{ marginBottom: 12, padding: '8px 10px', borderRadius: 9, background: 'var(--amber-tint, rgba(220,160,40,.14))', border: '1px solid var(--border)', fontSize: 'var(--text-caption)', color: 'var(--amber)', display: 'flex', gap: 6, alignItems: 'flex-start' }}>
            <Icon name="alert" size={13} style={{ flexShrink: 0, marginTop: 1 }} />
            <span>{candidate.conflict_hint}</span>
          </div>
        )}

        {/* 成员需求 + 选主需求（主需求驱动 CR 标题与召回） */}
        <div className="field-label" style={{ marginBottom: 6 }}>选择主需求（驱动变更标题）</div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4, marginBottom: 18 }}>
          {members.map(m => (
            <div key={m.id} onClick={() => setPrimary(m.id)}
              className="mention-row"
              style={{ cursor: 'pointer', border: '1px solid ' + (primary === m.id ? 'var(--ember)' : 'var(--border)'), borderRadius: 9, background: primary === m.id ? 'var(--ember-tint)' : 'transparent' }}>
              <span style={{ width: 14, height: 14, borderRadius: 99, flexShrink: 0, border: '2px solid ' + (primary === m.id ? 'var(--ember)' : 'var(--border-strong)'), background: primary === m.id ? 'var(--ember)' : 'transparent' }} />
              <div style={{ minWidth: 0, flex: 1 }}>
                <div className="nm" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={m.title}>{m.title}</div>
                <div style={{ fontSize: 'var(--text-micro)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)' }}>{m.id.slice(0, 8)}{primary === m.id ? ' · 主需求' : ''}</div>
              </div>
            </div>
          ))}
        </div>

        <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onCancel} disabled={busy}>取消</button>
          <button className="btn btn-primary" onClick={() => onConfirm(primary)} disabled={busy || !primary}>
            <Icon name={busy ? 'brain' : 'merge'} size={15} className={busy ? 'spin' : undefined} />
            {busy ? '合并中…' : '合并并进入编码'}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── ConflictResolver：逐 hunk 决策式三方解冲突器（方案 B）+ 外部 IDE 兜底（方案 C）──
type HunkChoice = { mode: 'ours' | 'theirs' | 'both' | 'manual'; manual?: string };
const HUNK_LABELS: Record<HunkChoice['mode'], string> = {
  ours: '采用本分支', theirs: '采用 dev', both: '两者保留', manual: '手动编辑',
};
// 冲突现场缓存（模块级，按 crId）：切换需求时命中即秒显，避免每次重读（后端要 git 物化冲突态，较慢）。
// 失效：解冲突收尾的 worktree_update 事件、或用户点「重读现场」/提交解决后——见 invalidateConflictCache。
const conflictDetailCache = new Map<string, ConflictDetail>();
const conflictViewCache = new Map<string, MergeConflictView>();
// 逐 hunk 决策也缓存：切走再切回时保留已做的选择，不丢工作。
const conflictChoiceCache = new Map<string, Record<string, HunkChoice>>();
function invalidateConflictCache(crId: string) {
  conflictDetailCache.delete(crId);
  conflictViewCache.delete(crId);
  conflictChoiceCache.delete(crId);
}
// 上下文折叠：超过 头+尾+1 行的无关上下文段，默认只显头/尾各若干行，中间一键展开。
const CTX_HEAD = 3, CTX_TAIL = 3;
function ConflictResolver({ crId, onAiResolve, onRefresh, showError, busy, resolving }: {
  crId: string; onAiResolve: () => void; onRefresh: () => void;
  showError: (m: string) => void; busy: boolean; resolving: boolean;
}) {
  const [detail, setDetail] = useState<ConflictDetail | null>(() => conflictDetailCache.get(crId) ?? null);
  const [loading, setLoading] = useState(false);
  const [activeFile, setActiveFile] = useState(0);
  const [choices, setChoices] = useState<Record<string, HunkChoice>>(() => conflictChoiceCache.get(crId) ?? {});
  const [submitting, setSubmitting] = useState(false);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});  // 已展开的上下文段（key=fi:si）
  const [reopened, setReopened] = useState<Record<string, boolean>>({});   // 已决策但又点开重新编辑的冲突块
  const scrollRef = useRef<HTMLDivElement>(null);
  const hunkRefs = useRef<Record<number, HTMLDivElement | null>>({});

  const load = useCallback((force = false) => {
    if (!force) {
      const cached = conflictDetailCache.get(crId);
      if (cached) {
        setDetail(cached); setChoices(conflictChoiceCache.get(crId) ?? {});
        setActiveFile(0); setExpanded({}); setReopened({}); setLoading(false);
        return;
      }
    }
    setLoading(true);
    getConflictDetail(crId)
      .then(d => {
        conflictDetailCache.set(crId, d);
        setDetail(d); setActiveFile(0); setChoices(conflictChoiceCache.get(crId) ?? {});
        setExpanded({}); setReopened({});
      })
      .catch(e => showError('读取冲突现场失败：' + String(e)))
      .finally(() => setLoading(false));
  }, [crId, showError]);
  useEffect(() => { load(); }, [load]);

  const segKey = (fi: number, si: number) => `${fi}:${si}`;
  const setChoice = (fi: number, si: number, c: HunkChoice) =>
    setChoices(prev => {
      const next = { ...prev, [segKey(fi, si)]: c };
      conflictChoiceCache.set(crId, next);  // 写穿缓存：切走再切回保留选择
      return next;
    });

  const preCode: React.CSSProperties = {
    margin: 0, whiteSpace: 'pre-wrap', wordBreak: 'break-word',
    fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-2)', lineHeight: 1.5,
  };

  // 上下文段渲染：长段折叠为头/尾各 CTX_* 行 + 「展开 N 行」，压缩无关代码。
  const renderContext = (text: string | null, si: number) => {
    if (!text) return null;
    const key = segKey(activeFile, si);
    const lines = text.split('\n');
    if (lines.length <= CTX_HEAD + CTX_TAIL + 1 || expanded[key]) {
      return <pre key={si} style={{ ...preCode, color: 'var(--text-faint)' }}>{text}</pre>;
    }
    const head = lines.slice(0, CTX_HEAD).join('\n');
    const tail = lines.slice(lines.length - CTX_TAIL).join('\n');
    const hidden = lines.length - CTX_HEAD - CTX_TAIL;
    return (
      <div key={si}>
        <pre style={{ ...preCode, color: 'var(--text-faint)' }}>{head}</pre>
        <button className="btn btn-sm btn-ghost" onClick={() => setExpanded(e => ({ ...e, [key]: true }))}
          style={{ width: '100%', justifyContent: 'center', color: 'var(--text-3)', fontSize: 'var(--text-micro)', margin: '3px 0', padding: '2px' }}>
          <Icon name="chevDown" size={12} />展开 {hidden} 行无关代码
        </button>
        <pre style={{ ...preCode, color: 'var(--text-faint)' }}>{tail}</pre>
      </div>
    );
  };

  if (loading && !detail) return <div className="empty-compact" style={{ padding: '16px 0' }}>读取冲突现场…</div>;
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
    try { await resolveConflictManually(crId, files); invalidateConflictCache(crId); onRefresh(); }
    catch (e) { showError('提交解决失败：' + String(e)); }
    finally { setSubmitting(false); }
  };
  const doExternalOpen = async () => {
    try { await openConflictWorkspace(crId); }
    catch (e) { showError('打开工作区失败：' + String(e)); }
  };
  const doExternalDone = async () => {
    setSubmitting(true);
    try { await resolveConflictManually(crId, null); invalidateConflictCache(crId); onRefresh(); }
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
          <button className="btn btn-primary btn-sm" disabled={busy || submitting} onClick={doExternalDone}><Icon name="check" size={13} />提交并复审</button>
          <button className="btn btn-sm" disabled={busy || submitting} onClick={onAiResolve}>
            <Icon name={resolving ? 'brain' : 'zap'} size={13} className={resolving ? 'spin' : undefined} />{resolving ? 'AI 解冲突中…' : 'AI 解冲突'}
          </button>
        </div>
      </div>
    );
  }

  const f = detail.files[activeFile];
  // 当前文件的冲突块（用于快速跳转编号 + 上一/下一）。
  const conflictList = f.binary ? [] : f.segments.map((s, si) => ({ s, si })).filter(x => x.s.kind === 'conflict');
  const jumpTo = (si: number) => hunkRefs.current[si]?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  return (
    <div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10 }}>
        <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', flex: 1 }}>
          逐处选择保留哪一侧（或两者保留 / 手动编辑）。已解决 <b style={{ color: 'var(--text-2)' }}>{decided}/{totalHunks}</b>。
        </div>
        <button className="btn btn-sm btn-ghost" disabled={loading || submitting} onClick={() => load(true)}
          title="重新读取冲突现场（默认走缓存，切换需求秒显）">
          <Icon name="refresh" size={13} className={loading ? 'spin' : undefined} />重读现场
        </button>
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
      {/* 快速跳转：当前文件有多个冲突块时，按编号一键滚动到对应块（已决策显示绿色）。 */}
      {conflictList.length > 1 && (
        <div style={{ display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: 6, marginBottom: 10 }}>
          <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-faint)', textTransform: 'uppercase', letterSpacing: '.1em' }}>跳转冲突</span>
          {conflictList.map((x, k) => {
            const done = !!choices[segKey(activeFile, x.si)];
            return (
              <button key={x.si} className={'chip ' + (done ? 'green' : '')} title={done ? '已决策' : '待决策'}
                style={{ cursor: 'pointer', fontSize: 'var(--text-micro)', minWidth: 22, justifyContent: 'center' }}
                onClick={() => jumpTo(x.si)}>{k + 1}</button>
            );
          })}
        </div>
      )}
      <div ref={scrollRef} style={{ background: 'var(--code-bg)', borderRadius: 8, padding: '10px 12px', maxHeight: 460, overflow: 'auto' }}>
        {f.binary ? (
          <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)' }}>二进制文件冲突，无法逐 hunk 解决，请用「外部编辑器打开」处理。</div>
        ) : f.segments.map((s, si) => {
          if (s.kind === 'context') return renderContext(s.text, si);
          const key = segKey(activeFile, si);
          const c = choices[key];
          const k = conflictList.findIndex(x => x.si === si) + 1;
          // 已决策（非手动）默认折叠为单行摘要，压缩内容；点「修改」展开重新编辑。
          const collapsed = !!c && c.mode !== 'manual' && !reopened[key];
          return (
            <div key={si} ref={el => { hunkRefs.current[si] = el; }}
              style={{ border: '1px solid ' + (c ? 'var(--border)' : 'var(--border-strong)'), borderLeft: '3px solid ' + (c ? 'var(--green)' : 'var(--amber)'), borderRadius: 8, margin: '8px 0', overflow: 'hidden' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '5px 8px', background: 'var(--bg-2)', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)' }}>
                <span style={{ color: c ? 'var(--green)' : 'var(--amber)', fontWeight: 700 }}>冲突 #{k}</span>
                {c && <span className="chip green" style={{ fontSize: 'var(--text-micro)' }}>{HUNK_LABELS[c.mode]}</span>}
                <span style={{ flex: 1 }} />
                {collapsed && (
                  <button className="btn btn-sm btn-ghost" style={{ padding: '1px 6px', fontSize: 'var(--text-micro)' }}
                    onClick={() => setReopened(r => ({ ...r, [key]: true }))}><Icon name="edit" size={12} />修改</button>
                )}
              </div>
              {!collapsed && (
                <>
                  <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 1, background: 'var(--border)' }}>
                    <div style={{ background: 'var(--bg-2)', padding: '6px 8px', opacity: c?.mode === 'ours' ? 0.45 : 1 }}>
                      <div style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '.1em', marginBottom: 4 }}>dev（传入）</div>
                      <pre style={preCode}>{s.theirs || '（空）'}</pre>
                    </div>
                    <div style={{ background: 'var(--bg-2)', padding: '6px 8px', opacity: c?.mode === 'theirs' ? 0.45 : 1 }}>
                      <div style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '.1em', marginBottom: 4 }}>本分支（你的）</div>
                      <pre style={preCode}>{s.ours || '（空）'}</pre>
                    </div>
                  </div>
                  <div style={{ display: 'flex', gap: 6, padding: '6px 8px', flexWrap: 'wrap', background: 'var(--bg-2)' }}>
                    {(['ours', 'theirs', 'both', 'manual'] as const).map(mode => (
                      <button key={mode} className={'chip ' + (c?.mode === mode ? 'ember' : '')} style={{ cursor: 'pointer', fontSize: 'var(--text-micro)' }}
                        onClick={() => { setChoice(activeFile, si, { mode, manual: mode === 'manual' ? (c?.manual ?? [s.ours, s.theirs].filter(x => x).join('\n')) : undefined }); if (mode !== 'manual') setReopened(r => ({ ...r, [key]: false })); }}>
                        {HUNK_LABELS[mode]}
                      </button>
                    ))}
                  </div>
                  {c?.mode === 'manual' && (
                    <textarea value={c.manual ?? ''} onChange={e => setChoice(activeFile, si, { mode: 'manual', manual: e.target.value })}
                      style={{ width: '100%', minHeight: 80, boxSizing: 'border-box', background: 'var(--bg-3)', border: 'none', borderTop: '1px solid var(--border)', color: 'var(--text)', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', padding: '6px 8px', resize: 'vertical' }} />
                  )}
                </>
              )}
            </div>
          );
        })}
      </div>
      <div style={{ display: 'flex', gap: 8, marginTop: 12, flexWrap: 'wrap' }}>
        <button className="btn btn-primary btn-sm" disabled={!allDecided || busy || submitting} onClick={doConfirm}><Icon name="check" size={13} />确认解决并复审</button>
        <button className="btn btn-sm" disabled={busy || submitting} onClick={onAiResolve}>
          <Icon name={resolving ? 'brain' : 'zap'} size={13} className={resolving ? 'spin' : undefined} />{resolving ? 'AI 解冲突中…' : 'AI 解冲突'}
        </button>
        <button className="btn btn-sm" disabled={busy || submitting} onClick={doExternalOpen}><Icon name="external" size={13} />外部编辑器打开</button>
        <button className="btn btn-sm" disabled={busy || submitting} onClick={doExternalDone}><Icon name="check" size={13} />我已在外部解决·复审</button>
      </div>
    </div>
  );
}

// ── LedgerView：全量需求总账（玻璃墙）──────────────────────────────────────────
// 只「看 / 下钻 / 整理」：所有状态可见 + 筛选搜索；状态只读、优先级不可拖；无拖拽/改状态/指派。
const LEDGER_STATUS_LABEL: Record<string, string> = {
  triage: '待整理', pending_analysis: '分析中', analysis_failed: '分析失败',
  pending_issue_review: '需求审核', pending_execution: '待编码', executing: '编码中',
  pending_code_review: '代码审核', pending_merge: '待合并', merge_testing: '合并中', merge_ready: '待落地', merged: '已合并',
  reverting: '撤销中', reverted: '已撤销',
  rejected: '已拒绝', deferred: '暂不处置', merge_failed: '合并失败', merge_conflict: '合并冲突', execution_failed: '执行失败', no_change_needed: '无需改动',
};
const LEDGER_STATUS_CHIP: Record<string, string> = {
  triage: '', pending_analysis: 'amber', analysis_failed: 'red', pending_issue_review: 'amber',
  executing: 'blue', pending_code_review: 'amber', merged: 'green', rejected: '', deferred: 'violet', merge_failed: 'red',
  merge_conflict: 'amber', no_change_needed: 'blue', reverting: 'amber', reverted: 'violet',
  pending_merge: 'blue', merge_testing: 'blue', merge_ready: 'amber',
};
// 不可拒绝的状态：运行中 / 待合并 / 测试中 / 待落地 / 已合并 / 已拒绝（与后端 reject_issues 跳过集一致）。
const REJECT_SKIP = ['executing', 'building', 'running', 'pending_execution', 'pending_merge', 'merge_testing', 'merge_ready', 'merged', 'rejected'];
const canReject = (s: string) => !REJECT_SKIP.includes(s);

function LedgerCheck({ on }: { on: boolean }) {
  return (
    <span style={{ width: 16, height: 16, borderRadius: 5, border: '1px solid ' + (on ? 'var(--ember)' : 'var(--border-strong)'), background: on ? 'var(--ember)' : 'var(--bg-3)', display: 'grid', placeItems: 'center' }}>
      {on && <Icon name="check" size={11} style={{ color: 'var(--bg)' }} />}
    </span>
  );
}

// 分组级三态勾选框：all=整组已选（实心勾）/ some=部分已选（横杠）/ none=未选（空框）。
function GroupCheck({ state }: { state: 'all' | 'some' | 'none' }) {
  const lit = state !== 'none';
  return (
    <span style={{ width: 16, height: 16, borderRadius: 5, border: '1px solid ' + (lit ? 'var(--ember)' : 'var(--border-strong)'), background: state === 'all' ? 'var(--ember)' : 'var(--bg-3)', display: 'grid', placeItems: 'center' }}>
      {state === 'all' && <Icon name="check" size={11} style={{ color: 'var(--bg)' }} />}
      {state === 'some' && <span style={{ width: 8, height: 2, borderRadius: 1, background: 'var(--ember)' }} />}
    </span>
  );
}

// 列表分组标题：整行点击折叠/展开（chevron 指示）；可批量分组左侧带「选择当前分组」三态框，
// 多个可批量分组并存时右侧再给一个「全选」按钮（跨组），二者语义区分。
function GroupHead({ label, count, collapsed, onToggleCollapse, group, all }: {
  label: string; count: number; collapsed: boolean; onToggleCollapse: () => void;
  group?: { state: 'all' | 'some' | 'none'; onToggle: () => void };
  all?: { selected: boolean; onToggle: () => void };
}) {
  return (
    <div className="req-group-head clickable" onClick={onToggleCollapse} style={{ cursor: 'pointer', userSelect: 'none' }}>
      <Icon name="chevRight" size={12} style={{ color: 'var(--text-faint)', transition: 'transform .15s', transform: collapsed ? 'none' : 'rotate(90deg)', flexShrink: 0 }} />
      {group && (
        <span onClick={e => { e.stopPropagation(); group.onToggle(); }}
          title={group.state === 'all' ? '取消选择当前分组' : '选择当前分组'}
          style={{ display: 'flex', flexShrink: 0, cursor: 'pointer' }}>
          <GroupCheck state={group.state} />
        </span>
      )}
      <span>{label}</span>
      <span style={{ fontFamily: 'var(--font-mono)', letterSpacing: 0, color: 'var(--text-3)' }}>{count}</span>
      {all && (
        <button onClick={e => { e.stopPropagation(); all.onToggle(); }}
          title={all.selected ? '取消全选（所有可批量分组）' : '全选所有可批量分组'}
          style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 4, background: 'none', border: 'none', cursor: 'pointer', padding: '2px 4px', borderRadius: 6, color: all.selected ? 'var(--ember-soft)' : 'var(--text-3)', fontSize: 'var(--text-caption)', fontFamily: 'var(--font-mono)', letterSpacing: 0, textTransform: 'none' }}>
          <Icon name="layers" size={12} /> {all.selected ? '取消全选' : '全选'}
        </button>
      )}
    </div>
  );
}

// 排序方向图示：上下双向箭头，激活方向（asc=上 / desc=下）以 ember 高亮，另一侧 faint。
// Icon 组件仅单色（currentColor），无法分侧上色，故用专用内联 SVG。
function SortGlyph({ asc }: { asc: boolean }) {
  const hot = 'var(--ember)';
  const cold = 'var(--text-faint)';
  return (
    <svg width="15" height="15" viewBox="0 0 16 16" fill="none" aria-hidden="true">
      <path d="M8 2.5 L11.5 6.5 L4.5 6.5 Z" fill={asc ? hot : cold} />
      <path d="M8 13.5 L11.5 9.5 L4.5 9.5 Z" fill={asc ? cold : hot} />
    </svg>
  );
}

const LEDGER_PAGE = 50;   // 总账每次滚动加载的条数
// 功能审计代码闸的分批加载：活动集（非合并）天然有界，一次取够；已合并历史按页滚动加载。
const CR_ACTIVE_CAP = 500;   // 活动集（非合并 CR）单次上限——远超工作流可能的在产数
const CR_MERGED_PAGE = 50;   // 已合并 CR 每次滚动加载的条数

function LedgerView({ projectId, refreshKey, sel, onSelectIssue, onRefineTriage, onRejectIssues, refiningIds, showMerged, onToggleMerged, mergedCount, initialStatus }: {
  projectId: string; refreshKey: number; sel: Sel | null; onSelectIssue: (id: string) => void;
  onRefineTriage: (ids: string[]) => Promise<void> | void;
  onRejectIssues: (ids: string[]) => Promise<void> | void;
  // 正在整理中的碎片 id：提升到父组件持有，使「整理中」状态在总账弹窗关闭/重开后仍保持。
  refiningIds: Set<string>;
  // 与功能审计页共享的「显示已合并需求」开关（默认隐藏）。
  showMerged: boolean; onToggleMerged: () => void; mergedCount: number;
  // 流水线节点跳转预置的初始状态筛选（缺省 'all'）。
  initialStatus?: string;
}) {
  const [search, setSearch] = useState('');
  const [dq, setDq] = useState('');            // 防抖后的查询串（喂给后端）
  // 创建时间排序方向：true=正序（最早在前），false=倒序（最新在前，默认——总账看全量数据，新需求置前）。
  const [sortAsc, setSortAsc] = useState(false);
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
  // 导出面板：全量 / 按状态类型多选导出（CSV / Excel）。
  const [exportOpen, setExportOpen] = useState(false);
  const [exportFmt, setExportFmt] = useState<'xlsx' | 'csv'>('xlsx');
  // 选中的导出状态类型（空集 = 全量导出，不按状态过滤）。
  const [exportSel, setExportSel] = useState<Set<string>>(new Set());
  const [exportSplit, setExportSplit] = useState(true);   // xlsx 按类型分表
  const [exporting, setExporting] = useState(false);
  const [exportMsg, setExportMsg] = useState<{ ok: boolean; text: string } | null>(null);
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
    listIssuesPage(projectId, statusFilter, dq, LEDGER_PAGE, 0, !showMerged, sortAsc)
      .then(p => { if (reqRef.current === token) { setItems(p.items); setTotal(p.total); } })
      .catch(() => { if (reqRef.current === token) { setItems([]); setTotal(0); } })
      .finally(() => { if (reqRef.current === token) setLoading(false); });
  }, [projectId, statusFilter, dq, refreshKey, showMerged, sortAsc]);

  const hasMore = items.length < total;
  const loadMore = useCallback(() => {
    if (loading || items.length >= total) return;
    const token = reqRef.current;  // 与当前重置同批；期间若发生重置则丢弃本次追加
    setLoading(true);
    listIssuesPage(projectId, statusFilter, dq, LEDGER_PAGE, items.length, !showMerged, sortAsc)
      .then(p => { if (reqRef.current === token) { setItems(prev => [...prev, ...p.items]); setTotal(p.total); } })
      .catch(() => {})
      .finally(() => { if (reqRef.current === token) setLoading(false); });
  }, [loading, items.length, total, projectId, statusFilter, dq, showMerged, sortAsc]);

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
  // 守卫，并发安全）。refiningIds 由父组件持有（跨弹窗关闭/重开保持），这里只过滤本批未在途
  // 的 id 交给父组件标记 spinner，跑完父组件摘除；本组件仅在结束后清选区。
  // 因此点一条整理时仍可点另一条同时跑。不占用全局 busy（busy 只留给拒绝等串行操作）。
  const runRefine = (ids: string[], clear: boolean) => {
    const fresh = ids.filter(id => !refiningIds.has(id));
    if (!fresh.length) return;
    Promise.resolve(onRefineTriage(fresh)).finally(() => {
      if (clear) setSelected(new Set());
    });
  };

  // 拒绝确认里区分 triage（硬删除）与其余（软归档），让用户清楚不可恢复的部分。
  const rejTriageCount = confirmReject ? confirmReject.ids.filter(id => items.find(i => i.id === id)?.status === 'triage').length : 0;

  // 导出：勾选的状态类型（空集 = 全量）。「显示已合并需求」关闭时不展示 merged 类型。
  const exportTypes = statuses.filter(s => showMerged || s !== 'merged');
  // 字段小标题样式（对齐 .field label：mono 大写小字）。
  const exportLabel: React.CSSProperties = { fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)', letterSpacing: '.04em', textTransform: 'uppercase' };
  const toggleExportType = (s: string) => setExportSel(prev => { const n = new Set(prev); n.has(s) ? n.delete(s) : n.add(s); return n; });
  const doExport = () => {
    if (exporting) return;
    setExporting(true);
    setExportMsg(null);
    const sel = [...exportSel].filter(s => exportTypes.includes(s));   // 仅导可见类型
    exportIssues(projectId, sel, exportFmt, exportFmt === 'xlsx' && exportSplit)
      .then(r => setExportMsg({ ok: true, text: `已导出 ${r.count} 条 → ${r.path}` }))
      .catch(e => setExportMsg({ ok: false, text: `导出失败：${String(e)}` }))
      .finally(() => setExporting(false));
  };

  return (
    <>
    <div style={{ display: 'flex', flexDirection: 'column', minHeight: 0, flex: 1 }}>
      {/* 搜索栏 + 筛选 tag：固定在顶部，不随列表滚动。 */}
      <div style={{ flexShrink: 0, display: 'flex', flexDirection: 'column', gap: 6, padding: '8px 12px' }}>
        <div style={{ display: 'flex', gap: 6, alignItems: 'center', position: 'relative' }}>
          <input value={search} onChange={e => setSearch(e.target.value)} placeholder="搜索标题 / 编号…"
            style={{ flex: 1, minWidth: 0, boxSizing: 'border-box', background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 8, padding: '6px 10px', color: 'var(--text)', fontSize: 'var(--text-control)', outline: 'none' }} />
          {/* 创建时间正/倒序切换：默认正序（旧需求在前），避免问题积压。 */}
          <button type="button" className="icon-btn" style={{ flexShrink: 0 }}
            onClick={() => setSortAsc(v => !v)}
            title={sortAsc ? '创建时间正序（最早在前）· 点击切换为倒序' : '创建时间倒序（最新在前）· 点击切换为正序'}>
            <SortGlyph asc={sortAsc} />
          </button>
          {/* 「显示已合并需求」开关：与功能审计页共享同一状态，默认隐藏。 */}
          <button className={'icon-btn' + (showMerged ? ' on' : '')} style={{ flexShrink: 0 }}
            onClick={onToggleMerged}
            title={showMerged ? `隐藏已合并需求（${mergedCount}）` : `显示已合并需求（${mergedCount}）`}>
            <Icon name={showMerged ? 'eye' : 'eye-off'} size={16} />
          </button>
          {/* 导出：全量 / 按状态类型多选导出（CSV / Excel）。 */}
          <button className={'icon-btn' + (exportOpen ? ' on' : '')} style={{ flexShrink: 0 }}
            onClick={() => { setExportOpen(v => !v); setExportMsg(null); }}
            title="导出需求（全量或按类型多选）">
            <Icon name="download" size={16} />
          </button>
        {exportOpen && (
          <div role="dialog" aria-label="导出需求"
            style={{ position: 'absolute', top: 'calc(100% + 6px)', right: 0, zIndex: 30, width: 320,
              display: 'flex', flexDirection: 'column',
              background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 'var(--radius)', boxShadow: 'var(--shadow-lg)', overflow: 'hidden' }}>
            {/* 头部 */}
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '11px 14px', borderBottom: '1px solid var(--border)' }}>
              <span style={{ display: 'flex', alignItems: 'center', gap: 7, fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', letterSpacing: '.1em', textTransform: 'uppercase', color: 'var(--text-2)' }}>
                <Icon name="download" size={14} style={{ color: 'var(--ember)' }} />导出需求
              </span>
              <button className="icon-btn btn-sm" onClick={() => setExportOpen(false)} title="关闭"><Icon name="x" size={15} /></button>
            </div>

            <div style={{ display: 'flex', flexDirection: 'column', gap: 14, padding: '14px' }}>
              {/* 格式 */}
              <div style={{ display: 'flex', flexDirection: 'column', gap: 7 }}>
                <span style={exportLabel}>格式</span>
                <div className="seg" style={{ alignSelf: 'flex-start' }}>
                  <button className={exportFmt === 'xlsx' ? 'on' : ''} onClick={() => setExportFmt('xlsx')}>Excel</button>
                  <button className={exportFmt === 'csv' ? 'on' : ''} onClick={() => setExportFmt('csv')}>CSV</button>
                </div>
              </div>

              {/* 类型多选（空选=全量） */}
              <div style={{ display: 'flex', flexDirection: 'column', gap: 7 }}>
                <div style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: 8 }}>
                  <span style={exportLabel}>类型</span>
                  <span style={{ fontSize: 'var(--text-micro)', color: exportSel.size ? 'var(--ember-soft)' : 'var(--text-faint)', fontFamily: 'var(--font-mono)' }}>
                    {exportSel.size === 0 ? '不选 = 全量' : `已选 ${exportSel.size} 类`}
                  </span>
                </div>
                <div style={{ display: 'flex', gap: 5, flexWrap: 'wrap' }}>
                  <button onClick={() => setExportSel(new Set())}
                    className={'filter-chip' + (exportSel.size === 0 ? ' on' : '')}
                    style={{ fontSize: 'var(--text-micro)', padding: '3px 9px' }}>全部</button>
                  {exportTypes.map(s => (
                    <button key={s} onClick={() => toggleExportType(s)}
                      className={'filter-chip' + (exportSel.has(s) ? ' on' : '')}
                      style={{ fontSize: 'var(--text-micro)', padding: '3px 9px' }}>
                      {LEDGER_STATUS_LABEL[s] ?? s}
                    </button>
                  ))}
                </div>
              </div>

              {/* xlsx 按类型分表 */}
              {exportFmt === 'xlsx' && (
                <label style={{ display: 'flex', alignItems: 'center', gap: 9, cursor: 'pointer' }} onClick={() => setExportSplit(v => !v)}>
                  <LedgerCheck on={exportSplit} />
                  <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-2)', lineHeight: 'var(--leading-snug)' }}>按类型分表<span style={{ color: 'var(--text-faint)' }}>（每个状态一个工作表）</span></span>
                </label>
              )}
            </div>

            {/* 底部操作 */}
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8, padding: '12px 14px', borderTop: '1px solid var(--border)', background: 'var(--bg-1)' }}>
              <button className="btn btn-sm btn-primary" disabled={exporting} onClick={doExport} title="导出到系统下载目录" style={{ width: '100%', justifyContent: 'center' }}>
                {exporting ? <><Icon name="brain" size={13} className="spin" />导出中…</> : <><Icon name="download" size={13} />导出{exportSel.size ? ` ${exportSel.size} 类` : '全量'}</>}
              </button>
              {exportMsg && (
                <span style={{ fontSize: 'var(--text-micro)', color: exportMsg.ok ? 'var(--green)' : 'var(--red)', fontFamily: 'var(--font-mono)', lineHeight: 'var(--leading-snug)', wordBreak: 'break-all' }} title={exportMsg.text}>
                  {exportMsg.text}
                </span>
              )}
            </div>
          </div>
        )}
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
      <div className="list-body scroll" ref={scrollRef} style={{ paddingTop: 0, flex: 1 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '6px 14px 6px', position: 'sticky', top: 0, zIndex: 2, background: 'var(--bg-1)', borderBottom: '1px solid var(--border)' }}>
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
            <span className="req-id" onClick={e => { e.stopPropagation(); toggle(i.id); }} style={{ cursor: 'pointer' }} title="点击选择">{i.id.slice(0, 8)}</span>
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
            <span className="req-time">{fmtShort(i.created_at)}</span>
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
  // spec 来自 LLM 输出，字段可能缺失或为空对象（如分析失败仅落 {} 占位）。任何字段都做
  // 防御性兜底——缺失绝不能让渲染抛错，否则 WebKitGTK 渲染进程崩溃会直接关闭窗口（主进程仍在）。
  const u = spec.understanding ?? ({} as Partial<NonNullable<IssueAnalysisSpec['understanding']>>);
  const rc = spec.root_cause;
  const sc = spec.scope ?? ({} as Partial<NonNullable<IssueAnalysisSpec['scope']>>);
  const plan = spec.implementation_plan ?? ({} as Partial<NonNullable<IssueAnalysisSpec['implementation_plan']>>);
  const b = spec.claude_code_brief ?? ({} as Partial<NonNullable<IssueAnalysisSpec['claude_code_brief']>>);
  const reproSteps = u.reproduction_steps ?? [];
  const affectedFiles = sc.affected_files ?? [];
  const relatedFiles = sc.related_files ?? [];
  const entryPoints = sc.entry_points ?? [];
  const outOfScope = sc.out_of_scope ?? [];
  const openQuestions = spec.open_questions ?? [];
  const acceptance = spec.acceptance_criteria ?? [];
  const mustList = spec.constraints?.must ?? [];
  const mustNotList = spec.constraints?.must_not ?? [];
  const risks = spec.risks ?? [];
  const instructions = b.instructions ?? [];
  const doList = b.do ?? [];
  const dontList = b.dont ?? [];
  const dod = b.definition_of_done ?? [];
  const dataModelChanges = plan.data_model_changes ?? [];
  const newDeps = plan.new_dependencies ?? [];
  const suspectedLocations = rc?.suspected_locations ?? [];
  const steps = [...(plan.steps ?? [])].sort((a, z) => a.order - z.order);

  const hasRoot = !!(rc && rc.hypothesis);
  const hasPlan = !!plan.approach || steps.length > 0;
  const hasAcceptance = acceptance.length > 0;
  const hasConstraints = mustList.length > 0 || mustNotList.length > 0;
  const hasRisks = risks.length > 0;
  const hasBrief = !!(b.objective || instructions.length > 0);
  const hasFull = hasRoot || hasPlan || hasAcceptance || hasConstraints || hasRisks || hasBrief;

  return (
    <>
      {/* ── 关键核心：默认可见 ── */}
      {((u.restated_issue || u.restated_requirement) || reproSteps.length > 0) && (
        <>
          <SpecH2 icon="search" color="var(--blue)">需求理解</SpecH2>
          {u.problem_type && <p style={{ margin: '0 0 8px' }}><span className="chip">{u.problem_type}</span></p>}
          {(u.restated_issue || u.restated_requirement) && <p style={{ whiteSpace: 'pre-line' }}>{u.restated_issue || u.restated_requirement}</p>}
          {u.current_behavior && <p style={liStyle}><b>当前行为：</b>{u.current_behavior}</p>}
          {u.expected_behavior && <p style={liStyle}><b>期望行为：</b>{u.expected_behavior}</p>}
          {reproSteps.length > 0 && (
            <ol style={{ paddingLeft: 18, margin: '6px 0', display: 'flex', flexDirection: 'column', gap: 3 }}>
              {reproSteps.map((s, i) => <li key={i} style={liStyle}>{s}</li>)}
            </ol>
          )}
        </>
      )}

      {(affectedFiles.length > 0 || relatedFiles.length > 0) && (
        <>
          <SpecH2 icon="file" color="var(--violet)">影响文件{sc.blast_radius ? <span className="chip" style={{ marginLeft: 8, fontSize: 'var(--text-micro)' }}>{sc.blast_radius}</span> : null}</SpecH2>
          {affectedFiles.length > 0 && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              {affectedFiles.map((f, i) => (
                <div key={i} style={{ display: 'flex', alignItems: 'baseline', gap: 8, flexWrap: 'wrap' }}>
                  <span className={'chip ' + (CHANGE_CHIP[f.change_type] || '')} style={{ fontSize: 'var(--text-micro)' }}>{f.change_type}</span>
                  <span style={monoPath}>{f.path}</span>
                  <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>{f.reason}</span>
                </div>
              ))}
            </div>
          )}
          {relatedFiles.length > 0 && (
            <div style={{ marginTop: affectedFiles.length > 0 ? 10 : 0 }}>
              <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', letterSpacing: '.06em', textTransform: 'uppercase', marginBottom: 4 }}>相关文件（需阅读，不一定改动）</div>
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
                {relatedFiles.map((p, i) => (
                  <span key={i} className="chip" style={{ ...monoPath, fontSize: 'var(--text-micro)' }}>{p}</span>
                ))}
              </div>
            </div>
          )}
          {entryPoints.length > 0 && <p style={{ ...liStyle, marginTop: 8 }}><b>入手点：</b>{entryPoints.join('；')}</p>}
          {outOfScope.length > 0 && <p style={liStyle}><b>不在范围：</b>{outOfScope.join('；')}</p>}
        </>
      )}

      {openQuestions.length > 0 && (
        <div className="iter-warn" style={{ marginTop: 14 }}>
          <Icon name="alert" size={20} />
          <div>
            <b>待澄清（批准前请确认）</b>
            <ul style={{ paddingLeft: 18, margin: '4px 0 0', display: 'flex', flexDirection: 'column', gap: 2 }}>
              {openQuestions.map((q, i) => <li key={i}>{q}</li>)}
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
                  {suspectedLocations.length > 0 && (
                    <div style={{ display: 'flex', flexDirection: 'column', gap: 6, margin: '6px 0' }}>
                      {suspectedLocations.map((l, i) => (
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
                  {dataModelChanges.filter(d => d.kind !== 'none' && d.description).map((d, i) => (
                    <p key={i} style={liStyle}><span className="chip violet" style={{ fontSize: 'var(--text-micro)' }}>{d.kind}</span> {d.description}</p>
                  ))}
                  {newDeps.length > 0 && <p style={liStyle}><b style={{ color: 'var(--amber)' }}>新增依赖：</b>{newDeps.join(', ')}</p>}
                </>
              )}

              {hasAcceptance && (
                <>
                  <SpecH2 icon="check" color="var(--green)">验收标准</SpecH2>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
                    {acceptance.map((ac, i) => (
                      <div key={i} style={liStyle}><span style={{ fontFamily: 'var(--font-mono)', color: 'var(--green)', fontSize: 'var(--text-caption)' }}>{ac.id}</span> {ac.statement}</div>
                    ))}
                  </div>
                </>
              )}

              {hasConstraints && (
                <>
                  <SpecH2 icon="shield" color="var(--blue)">约束</SpecH2>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                    {mustList.map((m, i) => <div key={'m' + i} style={liStyle}><span style={{ color: 'var(--green)' }}>✓</span> {m}</div>)}
                    {mustNotList.map((m, i) => <div key={'n' + i} style={liStyle}><span style={{ color: 'var(--red)' }}>✕</span> {m}</div>)}
                  </div>
                </>
              )}

              {hasRisks && (
                <>
                  <SpecH2 icon="alert" color="var(--red)">风险</SpecH2>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
                    {risks.map((r, i) => (
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
                  <SpecH2 icon="code" color="var(--text-3)">代码 Agent 执行工单</SpecH2>
                  <div className="panel" style={{ padding: '12px 14px' }}>
                    {b.objective && <p style={{ margin: '0 0 8px' }}><b>目标：</b>{b.objective}</p>}
                    {instructions.length > 0 && (
                      <ol style={{ paddingLeft: 18, margin: '0 0 8px', display: 'flex', flexDirection: 'column', gap: 3 }}>
                        {instructions.map((s, i) => <li key={i} style={liStyle}>{s}</li>)}
                      </ol>
                    )}
                    {doList.map((d, i) => <div key={'d' + i} style={liStyle}><span style={{ color: 'var(--green)' }}>✓</span> {d}</div>)}
                    {dontList.map((d, i) => <div key={'x' + i} style={liStyle}><span style={{ color: 'var(--red)' }}>✕</span> {d}</div>)}
                    {dod.length > 0 && (
                      <p style={{ ...liStyle, marginTop: 8 }}><b>完成判定：</b>{dod.join('；')}</p>
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

// 需求报告正文（描述 + 附件 + 验收 + 分析摘要/规格）——需求审核视图、全屏阅读、
// 全量总账的「详情查看」三处共用，保证被拒/搁置需求看到的内容与审核态完全一致。
function IssueReportBody({ issue, analysis, analysisLoading }: {
  issue: Issue; analysis: IssueAnalysis | null; analysisLoading: boolean;
}) {
  const analysisFailed = issue.status === 'analysis_failed';
  const spec = parseAnalysisSpec(analysis?.analysis_json);
  const notRecommended = spec?.triage?.needs_changes === false;
  return (
    <div className="report">
      <h2><Icon name="inbox" size={18} style={{ color: 'var(--ember)' }} />需求描述</h2>
      <p style={{ whiteSpace: 'pre-line' }}>{issue.description || '（无描述）'}</p>

      <BugCarrier issue={issue} />

      <h2><Icon name="paperclip" size={18} style={{ color: 'var(--ember)' }} />图片 / 附件</h2>
      <div style={{ marginBottom: 4 }}>
        <AttachmentBar issueId={issue.id} />
      </div>

      <AcceptancePanel issue={issue} />

      {analysisFailed && (
        <div className="chip red" style={{ display: 'block', padding: '12px 14px', margin: '12px 0', lineHeight: 'var(--leading-normal)' }}>
          <strong>自动分析失败</strong> · 可能是 LLM 超时、限流或未配置可用模型。已保留原始错误（见下方分析摘要），可点击右上角「重新分析」重试。
        </div>
      )}

      {issue.status === 'pending_analysis' && (
        <div className="chip blue" style={{ display: 'block', padding: '12px 14px', margin: '12px 0', lineHeight: 'var(--leading-normal)' }}>
          <strong>分析进行中</strong> · AI 正在分析该需求，完成后会自动进入「待需求审核」。{analysis ? '下方为上一轮分析结果，仅供参考。' : ''}
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
}

function IssueReviewView({ issue, analysis, analysisLoading, submitting, decided, advice, setAdvice, onDecide, onDefer, onRetryAnalysis, onReanalyze }: {
  issue: Issue; analysis: IssueAnalysis | null; analysisLoading: boolean;
  submitting: boolean; decided: string | null;
  advice: string; setAdvice: (v: string) => void;
  onDecide: (decision: 'approved' | 'rejected') => void;
  onDefer: () => void;
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

  // 需求描述 + 分析结果正文——内嵌视图与全屏阅读、详情查看（IssueDetailModal）共用同一组件。
  const renderReportBody = () => <IssueReportBody issue={issue} analysis={analysis} analysisLoading={analysisLoading} />;

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
          <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 2, display: 'flex', gap: 8, alignItems: 'center' }}>
            <span className={'chip ' + (SEV_COLOR[issue.category] || 'blue')} style={{ padding: '0 7px', fontSize: 'var(--text-micro)' }}>{issue.category}</span>
            <span className={'chip ' + (SEV_COLOR[issue.severity] || '')} style={{ padding: '0 7px', fontSize: 'var(--text-micro)' }}>{issue.severity}</span>
            {(() => { const s = issueSourceMeta(issue.source_type); return (
              <span className={'chip ' + s.chip} style={{ padding: '0 7px', fontSize: 'var(--text-micro)' }} title={`需求来源：${s.label}`}>
                <Icon name="inbox" size={11} style={{ marginRight: 3, opacity: .7 }} />来源 · {s.label}
              </span>
            ); })()}
            <span>{fmtFull(issue.created_at)}</span>
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
                  <button className="btn" onClick={onDefer} disabled={submitting}
                    title="暂不处置：搁置该需求，不进入编码；后续可在「全量需求总账」里重新分析（项目演进后重判）">
                    <Icon name="clock" size={15} />暂不处置
                  </button>
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

// ── IssueDetailModal（全量总账「详情查看」浮层）─────────────────────────────────
// 被拒 / 暂不处置 / 待整理等无审核闸口归宿的需求，在此只读查看完整内容（描述 + 附件 +
// 验收 + 分析），并对「已拒绝 / 暂不处置」提供重新启用入口。浮在总账之上；按 DESIGN，
// 关闭只走 ✕ / Esc（不点遮罩关闭）。
function IssueDetailModal({ issueId, onClose, onReactivated, showOk, showError }: {
  issueId: string;
  onClose: () => void;
  onReactivated: () => void;
  showOk: (m: string) => void;
  showError: (m: string) => void;
}) {
  const [issue, setIssue] = useState<Issue | null>(null);
  const [analysis, setAnalysis] = useState<IssueAnalysis | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    (async () => {
      const iss = await getIssue(issueId).catch(() => null);
      if (cancelled) return;
      setIssue(iss);
      const an = await getIssueAnalysis(issueId).catch(() => null);
      if (cancelled) return;
      setAnalysis(an);
      setLoading(false);
    })();
    return () => { cancelled = true; };
  }, [issueId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const reactivate = async (mode: 'reanalyze' | 'review') => {
    if (!issue) return;
    setBusy(true);
    try {
      await reactivateIssue(issue.id, mode);
      showOk(mode === 'reanalyze' ? '已重新启用 · 送回分析队列' : '已退回需求审核');
      onReactivated();
      onClose();
    } catch (e) {
      showError('重新启用失败：' + String(e));
    } finally { setBusy(false); }
  };

  const status = issue?.status;
  const hasAnalysis = !!analysis;
  return (
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.55)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 240 }}>
      <div style={{ width: 'min(860px, calc(100vw - 80px))', height: 'min(720px, calc(100vh - 64px))', background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
        <div className="audit-top" style={{ borderBottom: '1px solid var(--border)' }}>
          <div className="audit-top-info">
            <div className="audit-top-titlerow">
              <span className="req-id" style={{ fontSize: 'var(--text-control)' }}>{issueId.slice(0, 10)}</span>
              <CopyIdButton value={issueId} title="复制需求编号" />
              <span className="audit-top-title" style={{ fontWeight: 700, fontSize: 'var(--text-title)' }} title={issue?.title}>{issue?.title || '需求详情'}</span>
              {status && <span className={'chip ' + (STATUS_COLOR[status] ?? '')}>{STATUS_LABEL[status] ?? status}</span>}
            </div>
            {issue && (
              <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 2, display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
                <span className={'chip ' + (SEV_COLOR[issue.category] || 'blue')} style={{ padding: '0 7px', fontSize: 'var(--text-micro)' }}>{issue.category}</span>
                <span className={'chip ' + (SEV_COLOR[issue.severity] || '')} style={{ padding: '0 7px', fontSize: 'var(--text-micro)' }}>{issue.severity}</span>
                {(() => { const s = issueSourceMeta(issue.source_type); return (
                  <span className={'chip ' + s.chip} style={{ padding: '0 7px', fontSize: 'var(--text-micro)' }} title={`需求来源：${s.label}`}>
                    <Icon name="inbox" size={11} style={{ marginRight: 3, opacity: .7 }} />来源 · {s.label}
                  </span>
                ); })()}
                <span>{fmtFull(issue.created_at)}</span>
              </div>
            )}
          </div>
          <button className="icon-btn" title="关闭 (Esc)" onClick={onClose}><Icon name="x" size={18} /></button>
        </div>

        <div ref={scrollRef} className="diff-viewport scroll" style={{ flex: 1, minHeight: 0 }}>
          {loading
            ? <div className="empty-compact" style={{ padding: '40px 0' }}>加载中…</div>
            : issue
              ? <IssueReportBody issue={issue} analysis={analysis} analysisLoading={false} />
              : <div className="empty" style={{ flex: 1 }}><Icon name="alert" /><div>需求不存在或已被删除</div></div>}
        </div>

        {(status === 'rejected' || status === 'deferred') && (
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '12px 18px', borderTop: '1px solid var(--border)', background: 'var(--bg-1)' }}>
            <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>
              {status === 'deferred' ? '暂不处置 · 重启只能重新分析（项目可能已变化）' : '已拒绝 · 可重新启用'}
            </span>
            <span style={{ flex: 1 }} />
            {status === 'rejected' && hasAnalysis && (
              <button className="btn btn-sm" disabled={busy} onClick={() => reactivate('review')}
                title="沿用已有分析结果，直接退回「需求审核」闸口">
                <Icon name="refresh" size={14} />退回需求审核
              </button>
            )}
            <button className="btn btn-sm btn-primary" disabled={busy} onClick={() => reactivate('reanalyze')}
              title="回到分析队列重新分析，完成后进入「需求审核」">
              <Icon name="refresh" size={14} />{busy ? '处理中…' : '重新分析'}
            </button>
          </div>
        )}
      </div>
    </div>
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
  // 从总账下钻、但无审核闸口归宿的需求（已拒绝 / 暂不处置 / 待整理等）的「详情查看」浮层。
  const [detailIssueId, setDetailIssueId] = useState<string | null>(null);
  // 总账打开时的初始状态筛选：流水线节点跳到 triage/分析中/已合并等只读环节时预置。
  const [ledgerStatus, setLedgerStatus] = useState<string | undefined>(undefined);
  // 总账刷新信号：整理/拒绝后自增，触发总账重载首页（背景事件刷新不动它，避免浏览中被重置）。
  const [ledgerRefresh, setLedgerRefresh] = useState(0);
  // 正在整理中的碎片 id：取自模块级 refiningStore（脱离组件树），使「整理中」spinner 在
  // 总账弹窗关闭/重开、乃至切换页面后仍保持（后端 triage 命令本就在后台跑到完）。
  // 仅取当前项目的在途集合，做到项目间隔离——切到别的项目不再误显「整理中」。
  const refiningProjectId = activeProject?.id ?? '';
  const refiningSnapshot = useCallback(() => refiningStore.get(refiningProjectId), [refiningProjectId]);
  const refiningIds = useSyncExternalStore(refiningStore.subscribe, refiningSnapshot);
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
  // AI 解冲突是后台长任务：命令立即返回、CR 全程停在 merge_conflict，仅靠事件反映进度。
  // 记录「正在解冲突」的 CR id，给出持续的进行中指示（避免点完无反馈、以为没反应）。
  // 置位：点击 / 收到该 CR 的 resolving_conflict 事件（覆盖自动解冲突）；
  // 清除：收到该 CR 的 worktree_update 事件（finalize 成功/失败都发）。
  const [resolvingCrId, setResolvingCrId] = useState<string | null>(null);
  // 「撤销已合并需求」二次确认弹窗开关。
  const [revertConfirm, setRevertConfirm] = useState(false);
  const [revertBusy, setRevertBusy] = useState(false);
  const [restoreBusy, setRestoreBusy] = useState(false);
  const [grade, setGrade] = useState<CrGrade | null>(null);
  // 该 CR 覆盖的全部需求（合并 CR 含多条）——用于头部「覆盖 N 个需求」展示。
  const [crIssues, setCrIssues] = useState<CrIssueRef[]>([]);
  const [diffMode, setDiffMode] = useState<'unified' | 'split'>('unified');
  const [tab, setTab] = useState<'report' | 'diff' | 'logs'>('report');
  const [compareOpen, setCompareOpen] = useState(false);
  // 代码 Agent 执行日志：列表 meta + 当前展开的完整日志。
  const [runs, setRuns] = useState<CodeAgentRunMeta[]>([]);
  const [activeRun, setActiveRun] = useState<CodeAgentRunLog | null>(null);
  const [runStream, setRunStream] = useState<'stdout' | 'stderr'>('stdout');
  // 运行期实时日志（仅当前选中 CR）：直接累积成字符串（含换行/打字增量），切 CR 清空，
  // 运行结束后刷进 runs 落库列表。
  const [liveLog, setLiveLog] = useState<string>('');
  const liveEndRef = useRef<HTMLDivElement | null>(null);
  const [liveAutoScroll, setLiveAutoScroll] = useState(true); // 用户上滚查看历史时暂停自动滚底
  // 日志过滤：隐藏结果行（↳）/ 仅看发言（💬）。对实时与落库视图同时生效。
  const [hideResults, setHideResults] = useState(false);
  const [speechOnly, setSpeechOnly] = useState(false);
  // 一键复制日志的瞬时反馈：记当前刚复制的按钮 key（live/stdout/stderr），1.5s 后清。
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
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
  const [dockTab, setDockTab] = useState<'advice' | 'commit'>('advice');  // 底部 dock 右侧分段：管理员建议 / 合并信息（共用输入区，省空间）
  const [decided, setDecided] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  // 需求审核 / 代码审核 的「拒绝」均为不可逆决策，弹二次确认避免误点。
  const [confirmReject, setConfirmReject] = useState<null | 'review1' | 'review2'>(null);
  const [crLoading, setCrLoading] = useState(false);
  // 任务进度心跳：cr_id → 最近一次阶段说明，用于在编码/合并期间显示「活着」的进度。
  const [crProgress, setCrProgress] = useState<Record<string, { phase: string; note?: string }>>({});
  // 每项目两闸口待审计数：issue=待审核需求(pending_issue_review)，code=待审核代码(pending_code_review)。
  const [projectReviewCounts, setProjectReviewCounts] = useState<Record<string, { issue: number; code: number }>>({});
  const [intakeOpen, setIntakeOpen] = useState(false);
  // 日志内容经事件驱动累积（见 LiveLogModal）；phase 在渲染处按 sig 实时计算，故此处只存 sig。
  const [logModal, setLogModal] = useState<{ title: string; sig: string } | null>(null);
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
  // 微信小程序编译中（一次性 build，无持久进程）。crPhase 用 ref 读以保持回调稳定。
  const [crMiniappBuilding, setCrMiniappBuilding] = useState(false);
  const crMiniappBuildingRef = useRef(false);
  crMiniappBuildingRef.current = crMiniappBuilding;
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
    // 小程序：一次性编译，无持久进程；编译中=running 灯，否则停止。
    if (p.kind === 'miniapp') return crMiniappBuildingRef.current ? 'running' : 'stopped';
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
    const [pendingCrs, pendingIssueCounts] = await Promise.all([
      listChangeRequests(undefined, 'pending_code_review'),
      countPendingIssueReviews(),
    ]);
    const counts: Record<string, { issue: number; code: number }> = {};
    for (const c of pendingIssueCounts) {
      (counts[c.project_id] ??= { issue: 0, code: 0 }).issue = c.count;
    }
    for (const cr of pendingCrs) {
      (counts[cr.project_id] ??= { issue: 0, code: 0 }).code += 1;
    }
    setProjectReviewCounts(counts);
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
    // 需求审核 列表同时纳入「分析失败」与「分析中」需求：失败可一键重新分析，
    // 分析中（点了分析/重分析后）也保持可见，不再「点完就从列表消失」。
    const token = ++crReqRef.current;
    const [activePage, mergedPage, pending] = await Promise.all([
      listChangeRequestsPage(projectId, undefined, true, CR_ACTIVE_CAP, 0),
      listChangeRequestsPage(projectId, 'merged', false, CR_MERGED_PAGE, 0),
      listIssuesByStatuses(projectId, ['pending_issue_review', 'analysis_failed', 'pending_analysis']),
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
    // 整理归属当前项目；按项目隔离记录在途，切到别的项目不会误显「整理中」。
    const pid = activeProject?.id ?? '';
    if (!pid) return;
    // 只整理本批未在途的 id，避免重复入队；并入模块级 refiningStore 以驱动 spinner
    // （跨弹窗关闭/重开、跨页面切换均保持，因后端命令在后台跑到完）。
    const fresh = ids.filter(id => !refiningStore.get(pid).has(id));
    if (!fresh.length) return;
    refiningStore.add(pid, fresh);
    showInfo(`正在调用 triage Agent 整理 ${fresh.length} 条碎片，请稍候…`);
    try {
      const r = await refineTriage(fresh);
      if (r.errors && !r.refined && !r.discarded) {
        showError(`整理失败 ${r.errors} 条（请检查 triage 角色的 LLM 配置）`);
      } else {
        showOk(`整理完成：转入流水线 ${r.refined} · 判为噪音丢弃 ${r.discarded}` + (r.errors ? ` · 失败 ${r.errors}` : ''));
      }
    } catch (e) { showError('整理失败：' + String(e)); }
    finally { refiningStore.remove(pid, fresh); }
    setLedgerRefresh(v => v + 1);
    if (activeProject) await loadList(activeProject.id);
    // 整理会把 triage 碎片转为 pending_issue_review 等态，改变项目列表待审计数——刷新徽标与全局徽章。
    await loadProjectReviewCounts();
    window.dispatchEvent(new Event('AutoForge:badges-refresh'));
  }, [activeProject, loadList, loadProjectReviewCounts, showError, showOk, showInfo]);

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
    // 拒绝/归档会从待审队列移除需求，改变项目列表待审计数——刷新徽标与全局徽章（其余审核动作均如此）。
    await loadProjectReviewCounts();
    window.dispatchEvent(new Event('AutoForge:badges-refresh'));
  }, [activeProject, loadList, loadProjectReviewCounts, showError, showOk]);

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

  // 读取「自定义合并提交信息」开关（Settings 合并与放行面板）；关闭时审核页不显示输入框。
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
    // 预填默认合并信息（与后端回退模板一致），人审可改后随「批准合并」提交。
    // 先放短码占位避免空白，再异步拉取后端生成的 feat(模块): 标题 [autoforge #编号] 模板。
    setCommitMsg(`AutoForge merge: ${crId}`);
    getDefaultMergeMessage(crId)
      .then(msg => { if (loadReqRef.current === reqId && msg) setCommitMsg(msg); })
      .catch(() => {});
    setSession(null);   // 清掉上一份（含上一版本）报告，避免显示过期内容
    setCrIssues([]);    // 清掉上一条 CR 的覆盖需求列表
    setRuns([]); setActiveRun(null); setLiveLog(''); setLiveAutoScroll(true);  // 清掉上一条 CR 的执行日志 + 实时缓冲
    setDiff('');        // diff='' 时视图显示「加载中…」，重拉后替换
    setConflict(null);  // 清掉上一条 CR 的冲突现场
    autoOpenRef.current = false;  // 切换 CR 时取消上一条未完成的自动打开
    setCrAppLaunching(false);     // 切换 CR 时清掉上一条桌面应用「启动中」反馈
    setTimeout(() => adviceRef.current?.focus(), 120);

    // 报告页「需求原文」所需的原始需求：不再全量缓存，选中 CR 时按需补拉进 issuesById。
    const origIssueId = crs.find(c => c.id === crId)?.issue_id;
    (async () => {
      const [s, d, g, pv, origIssue, rl, cis] = await Promise.all([
        getWorktreeSession(crId),
        getCodeDiff(crId),
        getCrGrade(crId).catch(() => null),
        getCrPreview(crId).catch(() => null),
        origIssueId ? getIssue(origIssueId).catch(() => null) : Promise.resolve(null),
        listCodeAgentRuns(crId).catch(() => [] as CodeAgentRunMeta[]),
        getChangeRequestIssues(crId).catch(() => [] as CrIssueRef[]),
      ]);
      if (loadReqRef.current !== reqId) return;
      setSession(s);
      setCrPreview(pv);
      setDiff(d);
      setGrade(g);
      setRuns(rl);
      setCrIssues(cis);
      if (origIssue) setIssuesById(prev => ({ ...prev, [origIssue.id]: origIssue }));
      setCrLoading(false);
      // 合并冲突态：按需拉取冲突现场（文件列表 + 带标记 diff）供三方视图渲染。
      // 走模块级缓存，切换需求时命中即秒显，避免每次让后端重新 git 物化冲突态（较慢）。
      if (crs.find(c => c.id === crId)?.status === 'merge_conflict') {
        const cached = conflictViewCache.get(crId);
        if (cached) {
          setConflict(cached);
        } else {
          getMergeConflict(crId).then(c => { conflictViewCache.set(crId, c); if (loadReqRef.current === reqId) setConflict(c); }).catch(() => {});
        }
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
    listen<{ type?: string; cr_id?: string; phase?: string; note?: string }>('autoforge://event', e => {
      const ev = e.payload;
      // 进度心跳：即时更新（不防抖），让用户在长任务期间看到阶段流动。
      if (ev?.type === 'task_progress' && ev.cr_id) {
        setCrProgress(prev => ({ ...prev, [ev.cr_id as string]: { phase: ev.phase || '', note: ev.note } }));
        // AI 解冲突进行中：点亮该 CR 的「解决中」指示（也覆盖自动解冲突触发的场景）。
        if (ev.phase === 'resolving_conflict') setResolvingCrId(ev.cr_id);
        return;
      }
      // worktree_update 在解冲突收尾（成功/失败）时必发 → 收到即熄灭该 CR 的「解决中」指示，
      // 并失效其冲突现场缓存（现场已变/已消解，下次查看须重读）。
      if (ev?.type === 'worktree_update' && ev.cr_id) {
        setResolvingCrId(prev => (prev === ev.cr_id ? null : prev));
        invalidateConflictCache(ev.cr_id);
      }
      // 实时日志是高频事件，由专门的监听器处理，这里直接跳过——否则每段增量都会触发列表重载。
      if (ev?.type === 'code_agent_log') return;
      debounced();
    }).then(fn => { unlisten = fn; });
    return () => { if (timer) clearTimeout(timer); unlisten?.(); };
  }, [activeProject, loadList, loadProjectReviewCounts]);

  // 实时日志监听：累积当前选中 CR 的代码 Agent 输出。中途进入时 realtime 事件只能拿到订阅
  // 之后的增量（日志会从「现在」而非 00:00 开始），故订阅就绪后再拉一次运行中缓冲快照回灌已
  // 错过的开头，并按 chunk 序号去重无缝续接（快照含 [0, next_seq)，事件 seq≥next_seq 才追加）。
  // 运行结束（worktree_update）时刷新已落库列表，让完整日志接替实时缓冲。随 activeCr 重订阅。
  useEffect(() => {
    if (!activeCr) return;
    const cr = activeCr;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    let seeded = false;          // 快照是否已回灌；回灌前到达的增量先缓存，回灌后按序补放
    let seq = -1;                // 已并入 liveLog 的最高 chunk 序号
    const pending: { s: number; c: string }[] = [];
    const cap = (n: string) => (n.length > 400000 ? n.slice(-300000) : n);
    const apply = (s: number, c: string) => {
      if (s <= seq) return;      // 已包含（与快照或先到事件重叠）→ 跳过，避免重复
      seq = s;
      setLiveLog(prev => cap(prev + c));
    };
    listen<{ type?: string; cr_id?: string; stream?: string; chunk?: string; seq?: number }>('autoforge://event', e => {
      const ev = e.payload;
      if (ev?.cr_id !== cr) return;
      if (ev.type === 'code_agent_log' && ev.chunk) {
        const s = ev.seq ?? 0;
        if (!seeded) { pending.push({ s, c: ev.chunk }); return; }
        apply(s, ev.chunk);
      } else if (ev.type === 'worktree_update') {
        listCodeAgentRuns(cr).then(setRuns).catch(() => {});
      }
    }).then(fn => {
      if (cancelled) { fn(); return; }
      unlisten = fn;
      // 订阅就绪后再取快照：保证快照时间晚于订阅，中间增量必被事件捕获，按序号去重即可无缝衔接。
      const finish = (text: string, nextSeq: number) => {
        if (cancelled) return;
        if (text) setLiveLog(text);   // 直接置为快照全文（覆盖切 CR 时的空串）
        seq = nextSeq - 1;
        seeded = true;
        for (const p of pending) apply(p.s, p.c);
        pending.length = 0;
      };
      getRunningCodeAgentLog(cr)
        .then(snap => finish(snap.text, snap.next_seq))
        .catch(() => finish('', 0));
    });
    return () => { cancelled = true; unlisten?.(); };
  }, [activeCr]);

  // 实时日志自动滚到底（仅在执行日志 tab 打开、且用户未上滚查看历史时）。
  useEffect(() => {
    if (tab === 'logs' && liveLog.length > 0 && liveAutoScroll) liveEndRef.current?.scrollIntoView({ block: 'end' });
  }, [liveLog, tab, liveAutoScroll]);

  // 进入编码阶段（executing）自动切到「执行日志」tab，方便实时看 Agent 进度：
  // 选中正在执行的 CR、或当前 CR 状态刚变为 executing 时切换。用 ref 记上次 (id,status)，
  // 仅在「切到该 CR」或「状态刚变为 executing」时切一次，避免执行期间用户手动切走后被反复拉回。
  const selCrStatus = crs.find(c => c.id === activeCr)?.status;
  const prevExecRef = useRef<{ id: string; status?: string }>({ id: '' });
  useEffect(() => {
    const prev = prevExecRef.current;
    const enteredExec = selCrStatus === 'executing' && (prev.id !== activeCr || prev.status !== 'executing');
    prevExecRef.current = { id: activeCr, status: selCrStatus };
    if (enteredExec) setTab('logs');
  }, [activeCr, selCrStatus]);

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
  // 命令立即返回（后台长任务），故点击即点亮持续的「解决中」指示 + 即时提示，避免误以为没反应；
  // 真正结束由 worktree_update 事件熄灭指示。
  const doAiResolve = async () => {
    if (!activeCr || conflictBusy || resolvingCrId === activeCr) return;
    setConflictBusy(true);
    setResolvingCrId(activeCr);
    try {
      await aiResolveMergeConflict(activeCr);
      showOk('AI 解冲突已启动，正在后台处理（完成后回到代码审核复审）…');
      if (activeProject) await loadList(activeProject.id);
    } catch (e) {
      setResolvingCrId(prev => (prev === activeCr ? null : prev));
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

  // 已撤销需求闭环：恢复需求，重新进入执行队列（再次实现并经代码审核后合并）。
  const doRestore = async () => {
    if (!activeCr || restoreBusy) return;
    setRestoreBusy(true);
    try {
      await restoreChangeRequest(activeCr);
      if (activeProject) await loadList(activeProject.id);
      await loadProjectReviewCounts();
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    } catch (e) {
      showError('恢复需求失败：' + String(e));
    } finally { setRestoreBusy(false); }
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

  // 需求审核：暂不处置 → 搁置为 deferred（离开待审队列，可在总账里重新分析）。
  const doDefer = async () => {
    if (!activeIssueId || submitting) return;
    setSubmitting(true);
    try {
      await deferIssue(activeIssueId);
      setSel(null);
      if (activeProject) await loadList(activeProject.id);
      await loadProjectReviewCounts();
      setLedgerRefresh(v => v + 1);
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
      showOk('已暂不处置 · 需求已搁置（可在「全量需求总账」里重新分析）');
    } catch (e) {
      showError('暂不处置失败：' + String(e));
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

  // 合并需求（同文件多需求合并）：把多条需求合并成一个 CR + 一次编码执行。
  const doMergeReview1 = async (ids: string[], primaryId?: string) => {
    if (ids.length < 2) return;
    try {
      await review1Merge(ids, primaryId);
      showOk(`已合并 ${ids.length} 条需求为一次变更，进入编码`);
    } catch (e) {
      showError('合并失败：' + String(e));
    } finally {
      if (activeProject) await loadList(activeProject.id);
      await loadProjectReviewCounts();
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    }
  };

  // 需求审核（批量）：把选中的需求（待审核 / 分析失败）重新送回分析队列。无批量后端命令，
  // 逐条复用单条 retry_analysis（非法状态后端自行拦截，allSettled 不让单条失败影响整体）。
  const doBatchReanalyze = async (ids: string[]) => {
    if (!ids.length) return;
    const rs = await Promise.allSettled(ids.map(id => retryAnalysis(id)));
    const ok = rs.filter(r => r.status === 'fulfilled').length;
    const fail = rs.length - ok;
    showOk(`已重新分析：送回队列 ${ok}` + (fail ? ` · 跳过/失败 ${fail}` : ''));
    if (activeProject) await loadList(activeProject.id);
    await loadProjectReviewCounts();
    window.dispatchEvent(new Event('AutoForge:badges-refresh'));
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
    setLogModal({ title: `启动日志 · ${branch}`, sig: `branch:${pid}:${branch}` });
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
    setLogModal({ title: `启动日志 · ${branch}`, sig: `branch:${pid}:${branch}` });
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
      setLogModal({ title: '预览日志 · 本次改动', sig: `cr:${id}` });
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
    setLogModal({ title: '启动日志 · 桌面应用', sig: `cr:${id}` });
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

  // 微信小程序：一次性编译（无 dev server / 无端口 / 无浏览器）。编译完提示产物目录，
  // 用微信开发者工具打开。日志走同一份 cr:{id} 订阅，实时面板可见编译进度。
  const doBuildMiniapp = useCallback(async () => {
    if (!activeCr) return;
    const id = activeCr;
    setCrMiniappBuilding(true);
    setLogModal({ title: '编译日志 · 微信小程序', sig: `cr:${id}` });
    try {
      const res = await buildCrMiniapp(id);
      if (res.success) {
        if (res.launched_devtools) {
          showInfo(`编译成功 · 已用微信开发者工具打开产物目录：${res.artifact_dir}`);
        } else {
          showInfo(
            res.artifact_dir
              ? `编译成功 · 产物目录：${res.artifact_dir}（用微信开发者工具打开此目录预览；可在设置中配置 CLI 路径以自动打开）`
              : '编译成功，但未识别到产物目录，请查看编译日志确认输出位置。'
          );
        }
      } else {
        showError(`编译失败（退出码 ${res.exit_code}），请查看编译日志。`);
      }
    } catch (e) {
      showError('编译微信小程序失败：' + String(e));
    } finally {
      setCrMiniappBuilding(false);
    }
  }, [activeCr, showError, showInfo]);

  const showCrPreviewLog = useCallback(() => {
    if (!activeCr) return;
    const id = activeCr;
    setLogModal({ title: '预览日志 · 本次改动', sig: `cr:${id}` });
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
    if (kind === 'miniapp') {
      // 微信小程序：无 localhost server 可 iframe，预览=一次性编译产物。
      // 不轮询 reachability、不开浏览器；编译完提示产物目录，用微信开发者工具打开。
      return (
        <>
          <button className="btn btn-sm" disabled={crMiniappBuilding} onClick={doBuildMiniapp}>
            {crMiniappBuilding
              ? <><span className="dot amber" style={{ marginRight: 4 }} />编译中…</>
              : <><Icon name="box" size={14} />编译小程序</>}
          </button>
          <button className="btn btn-sm btn-ghost" onClick={showCrPreviewLog} title="查看编译日志">
            <Icon name="log" size={14} />
          </button>
        </>
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
              <span className={'chip ' + issueSourceMeta(oi.source_type).chip} style={{ fontSize: 'var(--text-micro)' }} title="需求来源">{issueSourceMeta(oi.source_type).label}</span>
              <span style={{ marginLeft: 'auto', fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{fmtFull(oi.created_at)}</span>
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
        (() => {
        const resolving = resolvingCrId === cr!.id;
        const busy = conflictBusy || resolving;
        return (
        <div style={{ background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderLeft: '3px solid var(--amber)', borderRadius: 10, padding: '14px 16px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10, color: 'var(--amber)', fontWeight: 700, fontSize: 'var(--text-body)' }}>
            <Icon name="alert" size={18} />{conflict && conflict.files.length > 0 ? '合并冲突 · 并入 dev 时发生代码冲突' : '合并受阻 · 并入 dev 后集成校验未通过'}
          </div>
          {/* AI 解冲突进行中：持续的进度横幅（后台长任务，全程停在 merge_conflict，靠它给反馈）。 */}
          {resolving && (
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12, padding: '10px 12px', borderRadius: 8, background: 'var(--ember-tint)', border: '1px solid var(--border-strong)', color: 'var(--text-2)', fontSize: 'var(--text-label)' }}>
              <Icon name="brain" size={15} className="spin" style={{ color: 'var(--ember)', flexShrink: 0 }} />
              <span>AI 正在后台解决冲突，完成后会自动回到代码审核复审。可在「执行日志」查看实时进度，期间无需操作。</span>
            </div>
          )}
          <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginBottom: 12 }}>
            {conflict && conflict.files.length > 0
              ? '该需求分支与 dev 上的其他改动冲突。可一键重试合并（dev 已含解时即可干净落地），或交由 AI 自动解决冲突（解完回到代码审核复审，不直接落 dev）。'
              : '该需求分支并入最新 dev 后测试未通过（集成破坏）。可一键重试合并，或交由 AI 修复后回到代码审核复审；也可在右上角「重新执行」基于最新 dev 重新实现。'}
          </div>
          {conflict && conflict.files.length > 0 ? (
            <>
              <ConflictResolver
                crId={cr!.id}
                busy={busy}
                resolving={resolving}
                onAiResolve={doAiResolve}
                onRefresh={() => { if (activeProject) loadList(activeProject.id); window.dispatchEvent(new Event('AutoForge:badges-refresh')); }}
                showError={showError}
              />
              <div style={{ marginTop: 10 }}>
                <button className="btn btn-sm" disabled={busy} onClick={doRetryMerge}>
                  <Icon name="refresh" size={13} />重试合并（dev 已含解时直接落地）
                </button>
              </div>
            </>
          ) : (
            <div style={{ display: 'flex', gap: 8 }}>
              <button className="btn btn-primary btn-sm" disabled={busy} onClick={doAiResolve}>
                <Icon name={resolving ? 'brain' : 'zap'} size={13} className={resolving ? 'spin' : undefined} />{resolving ? 'AI 解冲突中…' : 'AI 解冲突并合并'}
              </button>
              <button className="btn btn-sm" disabled={busy} onClick={doRetryMerge}>
                <Icon name="refresh" size={13} />重试合并
              </button>
            </div>
          )}
        </div>
        );
        })()
      ) : FAILED_STATUSES.includes(cr!.status) ? (
        <div style={{ background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderLeft: '3px solid var(--red)', borderRadius: 10, padding: '14px 16px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10, color: 'var(--red)', fontWeight: 700, fontSize: 'var(--text-body)' }}>
            <Icon name="alert" size={18} />{STATUS_LABEL[cr!.status] ?? '执行失败'}原因
          </div>
          <pre style={{ margin: 0, whiteSpace: 'pre-wrap', wordBreak: 'break-word', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-2)', lineHeight: 1.6 }}>
            {crLoading ? '加载中…' : (session?.report_content || '未捕获到失败详情，请重新执行或查看日志。')}
          </pre>
          {cr!.status === 'merge_failed' ? (
            <>
              <div style={{ marginTop: 12, fontSize: 'var(--text-label)', color: 'var(--text-3)' }}>
                该需求并入 dev 时合并/集成校验未通过。若已修复 dev 或判断为偶发，可「再次合并」（回到合并队列、在最新 dev 上重测后落地）；若需基于最新 dev 重建实现，则用右上角「重新执行」。
              </div>
              <div style={{ marginTop: 10 }}>
                <button className="btn btn-sm" disabled={conflictBusy} onClick={doRetryMerge}>
                  <Icon name="refresh" size={13} />再次合并
                </button>
              </div>
            </>
          ) : (
            <div style={{ marginTop: 12, fontSize: 'var(--text-label)', color: 'var(--text-3)' }}>
              可使用右上角「重新执行」重试，或「删除需求」清除该条异常数据。
            </div>
          )}
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

  // 点开一条执行日志：拉完整 stdout/stderr，默认展示有内容的那一路。再次点击已展开的同一条则收起。
  const openRun = async (id: string) => {
    if (activeRun?.id === id) { setActiveRun(null); return; }
    setActiveRun(null);
    try {
      const r = await getCodeAgentRun(id);
      setActiveRun(r);
      setRunStream(r && !r.stdout?.trim() && r.stderr?.trim() ? 'stderr' : 'stdout');
    } catch { /* 拉取失败静默：日志查看不阻断主流程 */ }
  };
  // 一键复制：写剪贴板并给 1.5s 的「已复制」反馈。剪贴板不可用时提示。
  const copyLog = async (text: string, key: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedKey(key);
      setTimeout(() => setCopiedKey(k => (k === key ? null : k)), 1500);
    } catch { showError('复制失败：剪贴板不可用'); }
  };
  const runExitMeta = (code: number) =>
    code === 0 ? { chip: 'green', label: '成功' }
      : code === 124 ? { chip: 'amber', label: '超时被杀' }
        : { chip: 'red', label: `退出码 ${code}` };
  const fmtBytes = (n: number) =>
    n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1048576).toFixed(2)} MB`;
  const RUN_PHASE_LABEL: Record<string, string> = { execution: '代码实现', conflict_resolve: 'AI 解冲突' };

  // 按过滤开关裁剪日志行：去掉行首 [mm:ss] 时间戳后按标记判定块类型；无标记的续行沿用
  // 上一块类型（多行发言/思考不会被拆散）。隐藏结果（↳）/ 仅看发言（💬）。
  const filterLogLines = (lines: { text: string; tone: string }[]) => {
    if (!hideResults && !speechOnly) return lines;
    let kind = '';
    return lines.filter(l => {
      const t = l.text.replace(/^\[\d{2}:\d{2}\]\s*/, '').trimStart();
      const m = t.startsWith('💬') ? 'speech'
        : t.startsWith('🔧') ? 'tool'
          : t.startsWith('↳') ? 'result'
            : t.startsWith('💭') ? 'think'
              : t.startsWith('●') ? 'sys'
                : (t.startsWith('✓') || t.startsWith('✗')) ? 'done' : '';
      if (m) kind = m;  // 有标记则更新当前块类型，续行不变
      if (speechOnly) return kind === 'speech';
      if (hideResults) return kind !== 'result';
      return true;
    });
  };
  // 统一渲染日志正文（解析 + 过滤 + 行号）。空时给占位。
  // 解析（含 ANSI 剥离 + 逐行正则判色）是日志渲染里最重的一步，按各来源 memo，
  // 避免无关重渲染（进度心跳、hover 等）反复全量重解析整段日志。
  const liveParsed = useMemo(() => parseLogLines(liveLog), [liveLog]);
  const stdoutParsed = useMemo(() => parseLogLines(activeRun?.stdout ?? ''), [activeRun]);
  const stderrParsed = useMemo(() => parseLogLines(activeRun?.stderr ?? ''), [activeRun]);
  const renderLogBody = (parsed: { text: string; tone: string }[], raw: string, emptyHint: string) => {
    if (!raw.trim()) return <div className="log-empty">{emptyHint}</div>;
    const lines = filterLogLines(parsed);
    if (lines.length === 0) return <div className="log-empty">（当前过滤下无内容）</div>;
    return lines.map((l, i) => (
      <LogLine key={i} n={i + 1} text={l.text} tone={l.tone} />
    ));
  };
  // 过滤开关条（实时与历史详情共用）。
  const renderLogFilters = () => (
    <div className="seg" style={{ flexShrink: 0 }}>
      <button className={hideResults ? 'on' : ''} onClick={() => { setHideResults(v => !v); setSpeechOnly(false); }}
        title="隐藏工具结果行（↳）">隐藏结果</button>
      <button className={speechOnly ? 'on' : ''} onClick={() => { setSpeechOnly(v => !v); setHideResults(false); }}
        title="只看 Agent 发言（💬）">仅发言</button>
    </div>
  );

  // 执行日志正文：实时输出（运行中）在最上，其下是历史运行列表（落库），点开看完整 stdout/stderr。
  const renderLogsBody = () => (
    <div className="report">
      {liveLog.length > 0 && (
        <div style={{ marginBottom: 14 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 6 }}>
            <span className="dot amber" />
            <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', color: 'var(--text-2)', textTransform: 'uppercase', letterSpacing: '.12em' }}>实时输出 · 运行中</span>
            <div style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 6 }}>
              {renderLogFilters()}
              {!liveAutoScroll && (
                <button className="btn btn-sm btn-ghost" onClick={() => setLiveAutoScroll(true)} title="恢复自动滚到底">
                  <Icon name="chevron" size={13} />跟随
                </button>
              )}
              <button className="btn btn-sm btn-ghost"
                onClick={() => copyLog(liveLog, 'live')} title="复制实时日志全文">
                <Icon name={copiedKey === 'live' ? 'check' : 'copy'} size={13} />{copiedKey === 'live' ? '已复制' : '复制'}
              </button>
            </div>
          </div>
          <div className="log-body scroll" style={{ border: '1px solid var(--ember)', borderRadius: 'var(--radius-sm)', maxHeight: '52vh' }}
            onScroll={e => {
              const el = e.currentTarget;
              setLiveAutoScroll(el.scrollHeight - el.scrollTop - el.clientHeight < 40);
            }}>
            {renderLogBody(liveParsed, liveLog, '（等待输出…）')}
            <div ref={liveEndRef} />
          </div>
        </div>
      )}
      {runs.length === 0 ? (
        liveLog.length === 0 && (
          <div className="empty-compact" style={{ padding: '20px 0' }}>
            {crLoading ? '加载中…' : '暂无执行日志（该需求尚未运行过代码 Agent，或日志已超出保留期）'}
          </div>
        )
      ) : (<>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6, marginBottom: 12 }}>
          {runs.map(r => {
            const ex = runExitMeta(r.exit_code);
            const on = activeRun?.id === r.id;
            return (
              <button key={r.id} onClick={() => openRun(r.id)}
                className="panel"
                style={{
                  display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap',
                  padding: '10px 12px', cursor: 'pointer', textAlign: 'left',
                  borderColor: on ? 'var(--ember)' : 'var(--border)',
                  background: on ? 'var(--ember-tint)' : 'var(--bg-2)',
                }}>
                <span className="chip ember" style={{ fontSize: 'var(--text-micro)' }}>{RUN_PHASE_LABEL[r.phase] || r.phase}</span>
                <span className="chip" style={{ fontSize: 'var(--text-micro)' }}>{r.kind}{r.model ? ` · ${r.model}` : ''}</span>
                <span className={'chip ' + ex.chip} style={{ fontSize: 'var(--text-micro)' }}>{ex.label}</span>
                <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>
                  {(r.duration_ms / 1000).toFixed(1)}s · out {fmtBytes(r.stdout_bytes)} · err {fmtBytes(r.stderr_bytes)}{r.truncated ? ' · 已截断' : ''}
                </span>
                <span style={{ marginLeft: 'auto', fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{fmtFull(r.created_at)}</span>
              </button>
            );
          })}
        </div>
        {activeRun && (<>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
            <div className="seg">
              <button className={runStream === 'stdout' ? 'on' : ''} onClick={() => setRunStream('stdout')}>stdout · {fmtBytes(activeRun.stdout_bytes)}</button>
              <button className={runStream === 'stderr' ? 'on' : ''} onClick={() => setRunStream('stderr')}>stderr · {fmtBytes(activeRun.stderr_bytes)}</button>
            </div>
            <div style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 6 }}>
              {runStream === 'stdout' && renderLogFilters()}
              <button className="btn btn-sm btn-ghost"
                onClick={() => copyLog(runStream === 'stdout' ? activeRun.stdout : activeRun.stderr, runStream)}
                title={`复制当前 ${runStream} 全文`}>
                <Icon name={copiedKey === runStream ? 'check' : 'copy'} size={13} />{copiedKey === runStream ? '已复制' : `复制 ${runStream}`}
              </button>
            </div>
          </div>
          {activeRun.truncated > 0 && (
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--amber)', marginBottom: 8 }}>
              <Icon name="alert" size={13} style={{ verticalAlign: -2, marginRight: 4 }} />日志过长，仅保留尾部约 512K 字符（开头已省略）。
            </div>
          )}
          <div className="log-body scroll" style={{ border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)' }}>
            {renderLogBody(runStream === 'stdout' ? stdoutParsed : stderrParsed, runStream === 'stdout' ? activeRun.stdout : activeRun.stderr, '（该流无输出）')}
          </div>
        </>)}
      </>)}
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
          onBatchReanalyze={doBatchReanalyze}
          onBatchReject={rejectIssuesItems}
          onMerge={doMergeReview1}
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
              onDefer={doDefer}
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
                    {crIssues.length > 1 && (
                      <span className="chip ember" title={crIssues.map(c => `${c.role === 'primary' ? '主 · ' : ''}${c.title}`).join('\n')}>
                        <Icon name="merge" size={12} />覆盖 {crIssues.length} 个需求
                      </span>
                    )}
                  </div>
                  <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 2, display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                    <span>{STATUS_LABEL[cr.status] ?? cr.status} · {fmtFull(cr.updated_at)}</span>
                    {(cr.status === 'executing' || cr.status === 'pending_merge' || cr.status === 'merge_testing' || cr.status === 'merge_ready') && crProgress[cr.id]?.note && (
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
                        {cr.status === 'reverted' && (
                          <button className="btn btn-primary" onClick={doRestore} disabled={restoreBusy} title="重新进入执行队列，再次实现并经代码审核后合并">
                            <Icon name="refresh" size={15} />{restoreBusy ? '恢复中…' : '恢复需求'}
                          </button>
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
                      <button className={tab === 'logs' ? 'on' : ''} onClick={() => setTab('logs')} style={{ position: 'relative' }}>
                        执行日志{runs.length > 0 ? ` · ${runs.length}` : ''}
                        {liveLog.length > 0 && <span className="dot amber" style={{ position: 'absolute', top: 3, right: 3 }} />}
                      </button>
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
                      {tab === 'diff' && (
                        <button className="btn btn-sm" title="与另一个 CR 并排对比 diff" onClick={() => setCompareOpen(true)}>
                          <Icon name="columns" size={13} />对比
                        </button>
                      )}
                      {tab !== 'logs' && (
                        <button className="icon-btn" title="全屏阅读（报告 + Diff 分栏）" onClick={() => setFsReader(true)}>
                          <Icon name="maximize" size={15} />
                        </button>
                      )}
                    </div>
                  </div>
                  <div className="diff-viewport scroll">
                    {/* AI 变更摘要卡片：报告 tab 顶部，基于 diff 实时生成（仅有实际改动的 CR）。 */}
                    {tab === 'report' && !NO_CHANGE_STATUSES.includes(cr.status) && !FAILED_STATUSES.includes(cr.status) && (
                      <ChangeSummaryCard crId={cr.id} enabled={!crLoading} />
                    )}
                    {/* 审核辅助：AI 代码预审摘要 + 发布说明（按需生成，仅有实际改动的 CR）。 */}
                    {tab === 'report' && !NO_CHANGE_STATUSES.includes(cr.status) && !FAILED_STATUSES.includes(cr.status) && (
                      <ReviewAssistCard crId={cr.id} enabled={!crLoading} />
                    )}
                    {/* 需求溯源时间线：录入→分析→审核→编码→合并 全链路追溯（折叠，按需加载）。 */}
                    {tab === 'report' && cr.issue_id && (
                      <LifecyclePanel issueId={cr.issue_id} />
                    )}
                    {tab === 'report' ? renderReportBody() : tab === 'diff' ? renderDiffBody() : renderLogsBody()}
                  </div>

                  {/* 底部悬浮 dock：左 = 本次改动预览启动；右 = 管理员建议 + 修改 */}
                  <div className="audit-dock">
                    {showCrPreview && (
                      <div className="dock-preview">
                        <span className="dock-label">本次改动预览</span>
                        <div className="dock-preview-actions">{renderCrLaunch()}</div>
                      </div>
                    )}
                    {(() => {
                      // 合并提交信息为可选项，仅在 Settings 开启自定义且 CR 待代码审核未决时可填。
                      // 它与管理员建议共用一块输入区，用 seg 分段切换，避免三列横向争抢把两者都压窄。
                      const showCommitTab = customMsgOn && cr.status === 'pending_code_review' && !decided;
                      const t = showCommitTab ? dockTab : 'advice';
                      return (
                        <div className="dock-advice">
                          {showCommitTab ? (
                            <div className="seg dock-seg">
                              <button className={t === 'advice' ? 'on' : ''} onClick={() => setDockTab('advice')}>管理员建议</button>
                              <button className={t === 'commit' ? 'on' : ''} onClick={() => setDockTab('commit')}>合并信息</button>
                            </div>
                          ) : (
                            <span className="dock-label">管理员建议 → 代码 Agent</span>
                          )}
                          <div className="dock-advice-row">
                            {t === 'commit' ? (
                              <input
                                value={commitMsg}
                                onChange={e => setCommitMsg(e.target.value)}
                                placeholder="留空则使用默认 AutoForge merge: <编号>"
                                title="批准合并时作为 merge --no-ff 的提交信息"
                              />
                            ) : (
                              <>
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
                              </>
                            )}
                          </div>
                        </div>
                      );
                    })()}
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
      {compareOpen && cr && (
        <CompareCrModal
          currentCrId={cr.id}
          currentLabel={issueTitles[cr.issue_id] || cr.id.slice(0, 8)}
          candidates={crs
            .filter(c => c.id !== cr.id)
            .map(c => ({ value: c.id, label: issueTitles[c.issue_id] || c.id.slice(0, 8) }))}
          onClose={() => setCompareOpen(false)}
        />
      )}

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
          <div style={{ width: 720, height: 'min(560px, calc(100vh - 40px))', background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', overflow: 'hidden', display: 'flex', flexDirection: 'column' }} onClick={e => e.stopPropagation()}>
            <div style={{ padding: '18px 20px 14px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
              <div className="eyebrow" style={{ fontSize: 'var(--text-section)' }}>
                <span className="cn">需求入口</span>
                <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginLeft: 8, fontFamily: 'var(--font-sans)', letterSpacing: 0, textTransform: 'none' }}>{activeProject.name}</span>
              </div>
              <button className="icon-btn" onClick={() => setIntakeOpen(false)}><Icon name="x" size={18} /></button>
            </div>
            <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
              <IntakePanel key={activeProject.id} projectId={activeProject.id} tabOrder={['bulk', 'manual', 'github', 'webhook']} />
            </div>
          </div>
        </div>
      )}

      {showLedger && (
        <div onMouseDown={() => setShowLedger(false)}
          style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 220 }}>
          <div onMouseDown={e => e.stopPropagation()}
            style={{ width: 'min(820px, calc(100vw - 64px))', height: 'min(680px, calc(100vh - 64px))', background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
            <div style={{ padding: '16px 20px 12px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
              <div className="eyebrow" style={{ fontSize: 'var(--text-section)', display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: 8 }}>
                <span className="cn">全量需求总账</span>
                {activeProject && <span className="chip ember">{activeProject.name}</span>}
                <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-sans)', letterSpacing: 0, textTransform: 'none' }}>所有状态 · 看 / 下钻 / 整理</span>
              </div>
              <button className="icon-btn" onClick={() => setShowLedger(false)}><Icon name="x" size={18} /></button>
            </div>
            <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
              <LedgerView projectId={activeProject?.id ?? ''} refreshKey={ledgerRefresh} sel={sel}
                onSelectIssue={async id => {
                  // 下钻到该需求并对齐审核闸口：有变更请求(已进入代码阶段，含已合并)→切「审核代码」选中其 CR；
                  // 仍在审核闸口的需求→切「审核需求」选中；其余无审核闸口归宿的状态（已拒绝 / 暂不处置 /
                  // 待整理 / 分析中等）→开「详情查看」浮层只读查看，避免被默认落位 effect 清掉而无处可看。
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
                  } else {
                    setDetailIssueId(id);
                  }
                }}
                onRefineTriage={refineTriageItems} onRejectIssues={rejectIssuesItems}
                refiningIds={refiningIds}
                showMerged={showMerged} onToggleMerged={() => setShowMerged(v => !v)}
                mergedCount={mergedTotal}
                initialStatus={ledgerStatus} />
            </div>
          </div>
        </div>
      )}

      {detailIssueId && (
        <IssueDetailModal
          issueId={detailIssueId}
          onClose={() => setDetailIssueId(null)}
          onReactivated={async () => {
            if (activeProject) await loadList(activeProject.id);
            await loadProjectReviewCounts();
            setLedgerRefresh(v => v + 1);
            window.dispatchEvent(new Event('AutoForge:badges-refresh'));
          }}
          showOk={showOk}
          showError={showError}
        />
      )}

      {logModal && (
        <LiveLogModal
          key={logModal.sig}
          title={logModal.title}
          sig={logModal.sig}
          // phase 在此实时计算：骑乘父组件已有的预览状态更新（crPreview / branchPreviews
          // 轮询），弹窗无需自行轮询即可让状态灯随进程「启动中→运行中→已退出」流转。
          phase={logModal.sig.startsWith('cr:')
            ? crPhase()
            : branchPhase(logModal.sig.replace(/^branch:[^:]*:/, ''))}
          onClose={() => setLogModal(null)}
        />
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
