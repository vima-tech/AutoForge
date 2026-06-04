import React, { useState, useEffect, useCallback, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import Icon from '../components/Icon';
import { ProjectCreateModal, ProjectEditModal, ConfirmProjectDeleteModal, parseProjectConfig } from '../components/ProjectDialogs';
import {
  listProjects, deleteProject, listChangeRequests, getWorktreeSession, getCodeDiff, review2,
  listPreviewEnvironments, seedDemoData, openUrl,
  getDevServerStatus, startDevServer, stopDevServer,
  type Project, type ChangeRequest, type WorktreeSession,
  type PreviewEnvironment, type DevServerStatus,
} from '../services';

const STATUS_LABEL: Record<string, string> = {
  pending_review_1: '待需求审核',
  executing: 'AI 执行中',
  pending_review_2: '待代码审核',
  merged: '已合并',
  rejected: '已拒绝',
};
const STATUS_COLOR: Record<string, string> = {
  pending_review_1: 'amber',
  executing: 'blue',
  pending_review_2: 'ember',
  merged: 'green',
  rejected: 'red',
};
const STATUS_ORDER = ['pending_review_2', 'executing', 'pending_review_1', 'merged', 'rejected'];

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

// ── PreviewSection ────────────────────────────────────────────────────────────

function PreviewSection({ label, tag, tagClass, url }: {
  label: string; tag: string; tagClass: string; url: string | null;
}) {
  const [reloadKey, setReloadKey] = useState(0);

  return (
    <div className="prev-section">
      <div className="prev-bar">
        <span className={`prev-tag ${tagClass}`}>{tag}</span>
        <span className="url">{url ?? '未配置'}</span>
        {url && (
          <>
            <button
              className="icon-btn" style={{ width: 24, height: 24 }}
              title="在浏览器打开"
              onClick={() => openUrl(url).catch(() => {})}
            >
              <Icon name="external" size={12} />
            </button>
            <button
              className="icon-btn" style={{ width: 24, height: 24 }}
              title="刷新"
              onClick={() => setReloadKey(k => k + 1)}
            >
              <Icon name="refresh" size={12} />
            </button>
          </>
        )}
      </div>
      {url
        ? <iframe key={reloadKey} src={url} className="prev-iframe" title={label} />
        : <div className="prev-empty">未配置预览环境</div>}
    </div>
  );
}

// ── AuditList ────────────────────────────────────────────────────────────────

function AuditList({ projects, activeProject, setActiveProject, projectReviewCounts, crs, activeCr,
  onSelect, onAddProject, onEditProject, onDeleteProject, devStatus, onStartServer, onStopServer, onSeedDemo, width }: {
  projects: Project[]; activeProject: Project | null; setActiveProject: (p: Project) => void;
  projectReviewCounts: Record<string, number>; crs: ChangeRequest[]; activeCr: string;
  onSelect: (id: string) => void; onAddProject: () => void; onEditProject: (p: Project) => void;
  onDeleteProject: (p: Project) => void;
  devStatus: DevServerStatus | null; onStartServer: () => void; onStopServer: () => void;
  onSeedDemo: () => void; width: number;
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
            <div className="proj-select" onClick={() => setOpen(o => !o)}>
              <div className="proj-logo" style={{ background: '#e8772e' }}>{activeProject.name[0]}</div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div className="proj-name">{activeProject.name}</div>
                <div className="proj-meta">{activeProject.description}</div>
              </div>
              <Icon name="chevDown" size={16} style={{ color: 'var(--text-3)', transition: 'transform .15s', transform: open ? 'rotate(180deg)' : 'none' }} />
            </div>
          ) : (
            <button className="btn btn-primary" style={{ justifyContent: 'center', width: '100%' }} onClick={onAddProject}>
              <Icon name="plus" size={15} />添加项目
            </button>
          )}
          {open && (
          <div className="mention-pop audit-project-pop" style={{ left: 0, right: 0, top: 'calc(100% + 6px)', bottom: 'auto', width: '100%', marginBottom: 0 }}>
            {projects.map(p => (
              <div key={p.id} className="mention-row" onClick={() => { setActiveProject(p); setOpen(false); }}>
                <div className="proj-logo" style={{ background: '#e8772e', width: 28, height: 28, fontSize: 12, borderRadius: 8 }}>{p.name[0]}</div>
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
                  <span className="chip amber" style={{ padding: '1px 6px', fontSize: 10, flexShrink: 0 }}>
                    {projectReviewCounts[p.id]}
                  </span>
                )}
                <div className="row-actions">
                  <button className="icon-btn" title="编辑项目" style={{ width: 24, height: 24 }}
                    onClick={e => { e.stopPropagation(); onEditProject(p); setOpen(false); }}>
                    <Icon name="edit" size={12} />
                  </button>
                  <button className="icon-btn" title="删除项目" style={{ width: 24, height: 24, color: 'var(--red)' }}
                    onClick={e => { e.stopPropagation(); onDeleteProject(p); setOpen(false); }}>
                    <Icon name="trash" size={12} />
                  </button>
                </div>
              </div>
            ))}
            <div style={{ height: 1, background: 'var(--border)', margin: '6px 4px' }} />
            <button className="btn btn-primary" style={{ width: '100%', justifyContent: 'center' }} onClick={() => { onAddProject(); setOpen(false); }}>
              <Icon name="plus" size={14} />添加项目
            </button>
          </div>
          )}
        </div>

        {devStatus?.status === 'running' ? (
          <button className="btn" style={{ justifyContent: 'flex-start', width: '100%', gap: 6 }}
            onClick={() => openUrl(devStatus.url!).catch(() => {})}>
            <span style={{ width: 7, height: 7, borderRadius: '50%', background: 'var(--green)', flexShrink: 0, display: 'inline-block' }} />
            <span style={{ fontFamily: 'var(--font-mono)', fontSize: 11, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1, textAlign: 'left' }}>
              {devStatus.url}
            </span>
            <button className="icon-btn" style={{ width: 22, height: 22, flexShrink: 0 }} title="停止服务"
              onClick={e => { e.stopPropagation(); onStopServer(); }}>
              <Icon name="x" size={11} />
            </button>
            <Icon name="external" size={13} style={{ color: 'var(--text-3)', flexShrink: 0 }} />
          </button>
        ) : devStatus?.status === 'starting' ? (
          <button className="btn" disabled style={{ justifyContent: 'center', width: '100%', gap: 6 }}>
            <span style={{ width: 7, height: 7, borderRadius: '50%', background: 'var(--amber)', flexShrink: 0, display: 'inline-block', animation: 'pulse 1.2s ease-in-out infinite' }} />
            <span style={{ fontFamily: 'var(--font-mono)', fontSize: 11, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1, textAlign: 'left' }}>
              {devStatus.url ?? '启动中…'}
            </span>
          </button>
        ) : devStatus?.status === 'idle' || devStatus?.status === 'stopped' ? (
          <button className="btn" style={{ justifyContent: 'center', width: '100%' }} onClick={onStartServer}>
            <Icon name="play" size={15} />启动项目
          </button>
        ) : activeProject ? (
          <div style={{ fontSize: 11.5, color: 'var(--text-faint)', textAlign: 'center', padding: '6px 8px', cursor: 'default' }}>
            未配置启动命令
          </div>
        ) : null}

        {activeProject && devStatus?.status !== 'running' && devStatus?.status !== 'starting' && (
          <button className="btn" style={{ justifyContent: 'center', width: '100%', fontSize: 11.5, color: 'var(--text-3)' }} onClick={onSeedDemo}>
            填充演示数据
          </button>
        )}
      </div>

      <div className="list-group-label">全部需求 · {crs.length} 条</div>
      <div className="list-body scroll" style={{ paddingTop: 0 }}>
        {crs.length === 0 && <div style={{ padding: '16px 12px', color: 'var(--text-3)', fontSize: 13 }}>暂无需求</div>}
        {(() => {
          const sorted = sortedCrs(crs);
          let lastStatus = '';
          return sorted.map(r => {
            const showLabel = r.status !== lastStatus;
            lastStatus = r.status;
            return (
              <React.Fragment key={r.id}>
                {showLabel && (
                  <div style={{ padding: '8px 12px 4px', fontSize: 10.5, letterSpacing: '.06em', textTransform: 'uppercase', color: 'var(--text-faint)', fontWeight: 600 }}>
                    {STATUS_LABEL[r.status] ?? r.status}
                  </div>
                )}
                <div className={'req-item' + (activeCr === r.id ? ' active' : '')} onClick={() => onSelect(r.id)}>
                  <div className="req-item-top">
                    <span className="req-id">{r.id.slice(0, 8)}</span>
                    <span className={'chip ' + (STATUS_COLOR[r.status] ?? '')} style={{ padding: '1px 7px', fontSize: 10 }}>{STATUS_LABEL[r.status] ?? r.status}</span>
                  </div>
                  <div className="req-title" style={{ fontSize: 13 }}>{r.issue_id.slice(0, 24)}</div>
                  <div className="req-foot">
                    <span style={{ marginLeft: 'auto' }}>{new Date(r.updated_at).toLocaleString('zh', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</span>
                  </div>
                </div>
              </React.Fragment>
            );
          });
        })()}
      </div>
    </div>
  );
}

// ── AuditPage ────────────────────────────────────────────────────────────────

export default function AuditPage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [activeProject, setActiveProject] = useState<Project | null>(null);
  const [crs, setCrs] = useState<ChangeRequest[]>([]);
  const [activeCr, setActiveCr] = useState('');
  const [session, setSession] = useState<WorktreeSession | null>(null);
  const [preview, setPreview] = useState<PreviewEnvironment | null>(null);
  const [devStatus, setDevStatus] = useState<DevServerStatus | null>(null);
  const [diff, setDiff] = useState('');
  const [diffMode, setDiffMode] = useState<'unified' | 'split'>('unified');
  const [tab, setTab] = useState<'report' | 'diff'>('report');
  const [advice, setAdvice] = useState('');
  const [decided, setDecided] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [crLoading, setCrLoading] = useState(false);
  const [projectReviewCounts, setProjectReviewCounts] = useState<Record<string, number>>({});
  const [showProjectCreate, setShowProjectCreate] = useState(false);
  const [projectToEdit, setProjectToEdit] = useState<Project | null>(null);
  const [projectToDelete, setProjectToDelete] = useState<Project | null>(null);
  const [projectError, setProjectError] = useState('');

  // Column widths
  const [listWidth, setListWidth] = useState(300);
  const [rightWidth, setRightWidth] = useState(420);

  // Advice textarea ref for auto-focus
  const adviceRef = useRef<HTMLTextAreaElement>(null);
  // Track which CR is being loaded to discard stale responses
  const loadingCrRef = useRef('');

  const loadProjectReviewCounts = useCallback(async () => {
    const pending = await listChangeRequests(undefined, 'pending_review_2');
    setProjectReviewCounts(pending.reduce<Record<string, number>>((acc, cr) => {
      acc[cr.project_id] = (acc[cr.project_id] ?? 0) + 1;
      return acc;
    }, {}));
  }, []);

  const loadProjects = useCallback(async () => {
    const ps = await listProjects();
    setProjects(ps);
    setActiveProject(cur => cur && ps.some(p => p.id === cur.id) ? cur : (ps[0] ?? null));
  }, []);

  useEffect(() => { loadProjects(); loadProjectReviewCounts(); }, [loadProjects, loadProjectReviewCounts]);

  // Load dev server status when active project changes
  useEffect(() => {
    if (!activeProject) { setDevStatus(null); return; }
    getDevServerStatus(activeProject.id).then(setDevStatus).catch(() => setDevStatus(null));
  }, [activeProject]);

  // Poll while server is starting
  useEffect(() => {
    if (!activeProject || devStatus?.status !== 'starting') return;
    const id = setInterval(() => {
      getDevServerStatus(activeProject.id).then(setDevStatus).catch(() => {});
    }, 2000);
    return () => clearInterval(id);
  }, [devStatus?.status, activeProject]);

  const loadCrs = useCallback(async (projectId: string) => {
    const all = await listChangeRequests(projectId);
    setCrs(all);
    if (all.length > 0 && !all.some(cr => cr.id === activeCr)) setActiveCr(sortedCrs(all)[0].id);
    if (all.length === 0) setActiveCr('');
  }, [activeCr]);

  useEffect(() => { if (activeProject) loadCrs(activeProject.id); }, [activeProject, loadCrs]);

  useEffect(() => {
    if (!activeCr) {
      setPreview(null);  // clear stale preview when no CR is selected
      return;
    }
    const crId = activeCr;
    loadingCrRef.current = crId;
    setCrLoading(true);
    setDecided(null);
    setAdvice('');
    setTimeout(() => adviceRef.current?.focus(), 120);

    (async () => {
      const [s, d] = await Promise.all([getWorktreeSession(crId), getCodeDiff(crId)]);
      if (loadingCrRef.current !== crId) return;
      let penv: PreviewEnvironment | null = null;
      if (s && activeProject) {
        const previews = await listPreviewEnvironments(activeProject.id);
        if (loadingCrRef.current !== crId) return;
        penv = previews.find(p => p.worktree_session_id === s.id && p.status !== 'terminated') ?? null;
      }
      setSession(s);
      setPreview(penv);
      setDiff(d);
      setCrLoading(false);
    })();
  }, [activeCr, activeProject]);

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const debounced = () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = null;
        if (activeProject) loadCrs(activeProject.id);
        loadProjectReviewCounts();
      }, 500);
    };
    let unlisten: (() => void) | undefined;
    listen('AutoForge://event', debounced).then(fn => { unlisten = fn; });
    return () => { if (timer) clearTimeout(timer); unlisten?.(); };
  }, [activeProject, loadCrs, loadProjectReviewCounts]);

  const doReview = async (decision: 'approved' | 'revision' | 'rejected') => {
    if (!activeCr || submitting) return;
    setSubmitting(true);
    try {
      await review2(activeCr, { decision, suggestions: advice || undefined });
      setDecided(decision);
      if (activeProject) await loadCrs(activeProject.id);
      await loadProjectReviewCounts();
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    } finally { setSubmitting(false); }
  };

  // 2. Enter 发送（Shift+Enter 换行）
  const onAdviceKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      if (canRevise && !submitting) doReview('revision');
    }
  };

  const doSeedDemo = useCallback(async () => {
    try {
      const msg = await seedDemoData();
      await loadProjects(); await loadProjectReviewCounts();
      if (activeProject) await loadCrs(activeProject.id);
      alert(msg);
    } catch (e) { alert('填充失败：' + String(e)); }
  }, [loadProjects, loadProjectReviewCounts, loadCrs, activeProject]);

  const doStartServer = useCallback(async () => {
    if (!activeProject) return;
    try {
      const status = await startDevServer(activeProject.id);
      setDevStatus(status);
    } catch (e) { alert('启动失败：' + String(e)); }
  }, [activeProject]);

  const doStopServer = useCallback(async () => {
    if (!activeProject) return;
    try {
      await stopDevServer(activeProject.id);
      setDevStatus(s => s ? { ...s, status: 'stopped' } : null);
    } catch (e) { alert('停止失败：' + String(e)); }
  }, [activeProject]);

  const doDeleteProject = async () => {
    if (!projectToDelete) return;
    setProjectError('');
    try {
      await deleteProject(projectToDelete.id);
      setProjectToDelete(null); setActiveCr('');
      await loadProjects(); await loadProjectReviewCounts();
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    } catch (e) { setProjectError(String(e)); setProjectToDelete(null); }
  };

  const cr = crs.find(c => c.id === activeCr);
  const report = session?.report_content ? parseReport(session.report_content) : null;
  const hunks = diff ? parseDiff(diff) : [];
  const canRevise = cr?.status === 'pending_review_2' && !decided;

  return (
    <>
      {showProjectCreate && (
        <ProjectCreateModal
          onClose={() => setShowProjectCreate(false)}
          onCreated={async project => {
            setShowProjectCreate(false);
            await loadProjects(); await loadProjectReviewCounts();
            setActiveProject(project); setActiveCr('');
          }}
        />
      )}
      {projectToEdit && (
        <ProjectEditModal
          project={projectToEdit}
          onClose={() => setProjectToEdit(null)}
          onSaved={async saved => {
            setProjectToEdit(null);
            await loadProjects();
            // If we edited the active project, refresh it and its dev status
            if (activeProject?.id === saved.id) {
              setActiveProject(saved);
              getDevServerStatus(saved.id).then(setDevStatus).catch(() => {});
            }
          }}
        />
      )}
      {projectToDelete && (
        <ConfirmProjectDeleteModal
          project={projectToDelete}
          onCancel={() => setProjectToDelete(null)}
          onConfirm={doDeleteProject}
        />
      )}

      {/* 1. 左侧列表 + 第一个拖拽分割线 */}
      <AuditList
        projects={projects} activeProject={activeProject}
        setActiveProject={p => { setActiveProject(p); setActiveCr(''); }}
        projectReviewCounts={projectReviewCounts} crs={crs} activeCr={activeCr}
        onSelect={id => { setActiveCr(id); setDecided(null); }}
        onAddProject={() => setShowProjectCreate(true)} onEditProject={setProjectToEdit}
        onDeleteProject={setProjectToDelete}
        devStatus={devStatus} onStartServer={doStartServer} onStopServer={doStopServer}
        onSeedDemo={doSeedDemo} width={listWidth}
      />
      <ResizeHandle onDrag={dx => setListWidth(w => Math.max(180, Math.min(520, w + dx)))} />

      <div className="content">
        {projectError && (
          <div style={{ padding: '10px 22px', color: 'var(--red)', fontSize: 13, borderBottom: '1px solid var(--border)' }}>
            {projectError}
          </div>
        )}
        {cr ? (
          <>
            {/* 顶部标题栏 */}
            <div className="audit-top">
              <div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <span className="req-id" style={{ fontSize: 13 }}>{cr.id.slice(0, 10)}</span>
                  <span style={{ fontWeight: 700, fontSize: 15 }}>Change Request</span>
                  {session && <span style={{ fontSize: 12, color: 'var(--text-3)' }}>迭代 {session.iteration_count} 轮</span>}
                </div>
                <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 2 }}>
                  {STATUS_LABEL[cr.status] ?? cr.status} · {new Date(cr.updated_at).toLocaleString('zh')}
                </div>
              </div>
              <div className="audit-decide">
                {cr.status !== 'pending_review_2'
                  ? <span className={'chip ' + (STATUS_COLOR[cr.status] ?? '')} style={{ padding: '7px 14px', fontSize: 13 }}>
                      {STATUS_LABEL[cr.status] ?? cr.status}
                    </span>
                  : decided
                    ? <span className={'chip ' + (decided === 'approved' ? 'green' : decided === 'rejected' ? 'red' : 'amber')} style={{ padding: '7px 14px', fontSize: 13 }}>
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
                        <div style={{ color: 'var(--text-3)', padding: '20px 0' }}>{session ? '报告内容为空' : '加载中…'}</div>
                      )}
                    </div>
                  ) : (
                    <div className="diff">
                      {hunks.length === 0
                        ? <div style={{ padding: '20px 22px', color: 'var(--text-3)' }}>{diff === '' ? '加载中…' : 'Diff 为空或 worktree 不存在'}</div>
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
              </div>

              {/* 1. 第二个拖拽分割线 */}
              <ResizeHandle onDrag={dx => setRightWidth(w => Math.max(200, Math.min(700, w - dx)))} />

              {/* 右栏：预览 + 建议 */}
              <div className="audit-right" style={{ width: rightWidth }}>
                <div className="prev-head">
                  <Icon name="eye" size={16} style={{ color: 'var(--ember)' }} />
                  <span style={{ fontWeight: 700, fontSize: 13.5 }}>实时预览对比</span>
                </div>
                <div className="prev-frames">
                  <PreviewSection label="生产 main" tag="生产 main" tagClass="prod" url={parseProjectConfig(activeProject?.config_yaml ?? null).prodUrl || null} />
                  <PreviewSection label={`本次改动 ${cr.id.slice(0, 8)}`} tag={`本次改动 ${cr.id.slice(0, 8)}`} tagClass="cr" url={preview?.preview_url ?? null} />
                </div>

                {/* 2. 管理员建议 + 修改按钮，Enter 发送 */}
                <div className="advice-wrap">
                  <div className="advice-label">管理员建议 → Claude Code</div>
                  <div className="advice-input">
                    <textarea
                      ref={adviceRef}
                      value={advice}
                      onChange={e => setAdvice(e.target.value)}
                      onKeyDown={onAdviceKeyDown}
                      placeholder={canRevise ? '输入修改意见，Enter 发送，Shift+Enter 换行…' : '输入备注（只读状态不会提交）…'}
                    />
                    {canRevise && (
                      <div className="advice-input-foot">
                        <button className="btn" style={{ padding: '5px 12px', fontSize: 12 }} onClick={() => doReview('revision')} disabled={submitting}>
                          <Icon name="refresh" size={14} />修改
                        </button>
                      </div>
                    )}
                  </div>
                </div>
              </div>
            </div>
          </>
        ) : (
          <div className="empty" style={{ height: '100%' }}><Icon name="audit" /><div>选择一个需求查看详情</div></div>
        )}
      </div>
    </>
  );
}
