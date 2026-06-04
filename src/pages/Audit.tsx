import React, { useState, useEffect, useCallback } from 'react';
import { listen } from '@tauri-apps/api/event';
import Icon from '../components/Icon';
import { ProjectCreateModal, ConfirmProjectDeleteModal } from '../components/ProjectDialogs';
import {
  listProjects, deleteProject, listChangeRequests, getWorktreeSession, getCodeDiff, review2,
  listPreviewEnvironments,
  type Project, type ChangeRequest, type WorktreeSession,
  type PreviewEnvironment,
} from '../services';

const CAT_COLOR: Record<string, string> = { Bug:'red', Feature:'ember', Improvement:'blue', Debt:'violet' };

// Parse report_content into sections
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

// Parse unified diff text into hunks
interface DiffLine { n1: string|number; n2: string|number; t: 'add'|'del'|'ctx'; code: string }
interface Hunk { file: string; hunk: string; lines: DiffLine[] }
function parseDiff(raw: string): Hunk[] {
  const hunks: Hunk[] = [];
  let curFile = '';
  let curHunk: Hunk | null = null;
  for (const line of raw.split('\n')) {
    if (line.startsWith('--- ')) { curFile = line.replace('--- ','').replace('a/',''); continue; }
    if (line.startsWith('+++ ')) continue;
    if (line.startsWith('@@ ')) {
      curHunk = { file: curFile, hunk: line, lines: [] };
      hunks.push(curHunk);
      continue;
    }
    if (!curHunk) continue;
    if (line.startsWith('+')) curHunk.lines.push({ n1: '', n2: '', t: 'add', code: line.slice(1) });
    else if (line.startsWith('-')) curHunk.lines.push({ n1: '', n2: '', t: 'del', code: line.slice(1) });
    else curHunk.lines.push({ n1: '', n2: '', t: 'ctx', code: line.slice(1) });
  }
  return hunks;
}

function AuditList({ projects, activeProject, setActiveProject, projectReviewCounts, crs, activeCr, onSelect, onAddProject, onDeleteProject }: {
  projects: Project[]; activeProject: Project | null; setActiveProject: (p: Project) => void;
  projectReviewCounts: Record<string, number>; crs: ChangeRequest[]; activeCr: string; onSelect: (id: string) => void;
  onAddProject: () => void; onDeleteProject: (project: Project) => void;
}) {
  const [open, setOpen] = useState(false);
  const projectMenuRef = React.useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;

    const closeIfOutside = (e: PointerEvent) => {
      const target = e.target;
      if (!(target instanceof Node)) return;
      if (projectMenuRef.current?.contains(target)) return;
      setOpen(false);
    };

    document.addEventListener('pointerdown', closeIfOutside);
    return () => document.removeEventListener('pointerdown', closeIfOutside);
  }, [open]);

  return (
    <div className="list-col">
      <div className="audit-proj" ref={projectMenuRef}>
        {activeProject && (
          <div className="proj-select" onClick={() => setOpen(o => !o)}>
            <div className="proj-logo" style={{ background: '#e8772e' }}>{activeProject.name[0]}</div>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div className="proj-name">{activeProject.name}</div>
              <div className="proj-meta">{activeProject.description}</div>
            </div>
            <Icon name="chevDown" size={16} style={{ color: 'var(--text-3)' }} />
          </div>
        )}
        {!activeProject && (
          <button className="btn btn-primary" style={{ justifyContent:'center', width:'100%' }} onClick={onAddProject}>
            <Icon name="plus" size={15} />添加项目
          </button>
        )}
        {open && (
          <div className="mention-pop audit-project-pop" style={{ left: 16, top: 64, bottom: 'auto', width: 'calc(var(--list-w) - 32px)', marginBottom: 0 }}>
            {projects.map(p => (
              <div key={p.id} className="mention-row" onClick={() => { setActiveProject(p); setOpen(false); }}>
                <div className="proj-logo" style={{ background:'#e8772e',width:30,height:30,fontSize:13 }}>{p.name[0]}</div>
                <div style={{ minWidth: 0, flex: 1 }}>
                  <div className="nm">{p.name}</div>
                  <div className="rl">{p.description || p.slug}</div>
                </div>
                <span className={'chip ' + ((projectReviewCounts[p.id] ?? 0) > 0 ? 'amber' : '')} style={{ padding:'1px 7px',fontSize:10 }}>
                  {projectReviewCounts[p.id] ?? 0} 待审
                </span>
                {p.id === activeProject?.id && <Icon name="check" size={15} style={{ color:'var(--ember)' }} />}
                <button className="icon-btn" title="删除项目" style={{ width: 26, height: 26, color: 'var(--red)' }} onClick={(e) => { e.stopPropagation(); onDeleteProject(p); setOpen(false); }}>
                  <Icon name="trash" size={13} />
                </button>
              </div>
            ))}
            <div style={{ height: 1, background: 'var(--border)', margin: '6px 4px' }} />
            <button className="btn btn-primary" style={{ width: '100%', justifyContent: 'center' }} onClick={() => { onAddProject(); setOpen(false); }}>
              <Icon name="plus" size={14} />添加项目
            </button>
          </div>
        )}
        <button className="btn" style={{ justifyContent:'center', width:'100%' }}><Icon name="play" size={15} />预览记录已接入</button>
      </div>
      <div className="list-group-label">待审核需求 · 审核节点 2</div>
      <div className="list-body scroll" style={{ paddingTop: 0 }}>
        {crs.length === 0 && <div style={{ padding:'16px 12px',color:'var(--text-3)',fontSize:13 }}>暂无待审核需求</div>}
        {crs.map(r => (
          <div key={r.id} className={'req-item'+(activeCr===r.id?' active':'')} onClick={() => onSelect(r.id)}>
            <div className="req-item-top">
              <span className="req-id">{r.id.slice(0,10)}</span>
              <span className={'chip '+CAT_COLOR['Feature']} style={{ padding:'1px 7px',fontSize:10 }}>CR</span>
            </div>
            <div className="req-title" style={{ fontSize:13 }}>{r.id}</div>
            <div className="req-foot"><span style={{ marginLeft:'auto' }}>{new Date(r.updated_at).toLocaleString('zh',{month:'numeric',day:'numeric',hour:'2-digit',minute:'2-digit'})}</span></div>
          </div>
        ))}
      </div>
    </div>
  );
}

export default function AuditPage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [activeProject, setActiveProject] = useState<Project | null>(null);
  const [crs, setCrs] = useState<ChangeRequest[]>([]);
  const [activeCr, setActiveCr] = useState('');
  const [session, setSession] = useState<WorktreeSession | null>(null);
  const [preview, setPreview] = useState<PreviewEnvironment | null>(null);
  const [diff, setDiff] = useState('');
  const [diffMode, setDiffMode] = useState<'unified'|'split'>('unified');
  const [tab, setTab] = useState<'report'|'diff'>('report');
  const [advice, setAdvice] = useState('');
  const [decided, setDecided] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [projectReviewCounts, setProjectReviewCounts] = useState<Record<string, number>>({});
  const [showProjectCreate, setShowProjectCreate] = useState(false);
  const [projectToDelete, setProjectToDelete] = useState<Project | null>(null);
  const [projectError, setProjectError] = useState('');

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
    setActiveProject(current => current && ps.some(p => p.id === current.id) ? current : (ps[0] ?? null));
  }, []);

  useEffect(() => {
    loadProjects();
    loadProjectReviewCounts();
  }, [loadProjects, loadProjectReviewCounts]);

  const loadCrs = useCallback(async (projectId: string) => {
    const all = await listChangeRequests(projectId, 'pending_review_2');
    setCrs(all);
    if (all.length > 0 && !all.some(cr => cr.id === activeCr)) setActiveCr(all[0].id);
    if (all.length === 0) setActiveCr('');
  }, [activeCr]);

  useEffect(() => { if (activeProject) loadCrs(activeProject.id); }, [activeProject, loadCrs]);

  useEffect(() => {
    if (!activeCr) return;
    setSession(null); setPreview(null); setDiff('');
    getWorktreeSession(activeCr).then(async s => {
      setSession(s);
      if (s && activeProject) {
        const previews = await listPreviewEnvironments(activeProject.id);
        setPreview(previews.find(p => p.worktree_session_id === s.id && p.status !== 'terminated') ?? null);
      }
    });
    getCodeDiff(activeCr).then(setDiff);
    setDecided(null); setAdvice('');
  }, [activeCr, activeProject]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen('autoforge://event', () => {
      if (activeProject) loadCrs(activeProject.id);
      loadProjectReviewCounts();
    }).then(fn => { unlisten = fn; });
    return () => unlisten?.();
  }, [activeProject, loadCrs, loadProjectReviewCounts]);

  const doReview = async (decision: 'approved'|'revision'|'rejected') => {
    if (!activeCr || submitting) return;
    setSubmitting(true);
    try {
      await review2(activeCr, { decision, suggestions: advice || undefined });
      setDecided(decision);
      if (activeProject) await loadCrs(activeProject.id);
      await loadProjectReviewCounts();
      window.dispatchEvent(new Event('autoforge:badges-refresh'));
    } finally { setSubmitting(false); }
  };

  const doDeleteProject = async () => {
    if (!projectToDelete) return;
    setProjectError('');
    try {
      await deleteProject(projectToDelete.id);
      setProjectToDelete(null);
      setActiveCr('');
      await loadProjects();
      await loadProjectReviewCounts();
      window.dispatchEvent(new Event('autoforge:badges-refresh'));
    } catch (e) {
      setProjectError(String(e));
      setProjectToDelete(null);
    }
  };

  const cr = crs.find(c => c.id === activeCr);
  const report = session?.report_content ? parseReport(session.report_content) : null;
  const hunks = diff ? parseDiff(diff) : [];

  return (
    <>
      {showProjectCreate && (
        <ProjectCreateModal
          onClose={() => setShowProjectCreate(false)}
          onCreated={async (project) => {
            setShowProjectCreate(false);
            await loadProjects();
            await loadProjectReviewCounts();
            setActiveProject(project);
            setActiveCr('');
          }}
        />
      )}
      {projectToDelete && <ConfirmProjectDeleteModal project={projectToDelete} onCancel={() => setProjectToDelete(null)} onConfirm={doDeleteProject} />}
      <AuditList projects={projects} activeProject={activeProject} setActiveProject={p => { setActiveProject(p); setActiveCr(''); }} projectReviewCounts={projectReviewCounts} crs={crs} activeCr={activeCr} onSelect={id => { setActiveCr(id); setDecided(null); }} onAddProject={() => setShowProjectCreate(true)} onDeleteProject={setProjectToDelete} />
      <div className="content">
        {projectError && <div style={{ padding: '10px 22px', color: 'var(--red)', fontSize: 13, borderBottom: '1px solid var(--border)' }}>{projectError}</div>}
        {cr ? (
          <>
            <div className="audit-top">
              <div>
                <div style={{ display:'flex',alignItems:'center',gap:8 }}>
                  <span className="req-id" style={{ fontSize:13 }}>{cr.id.slice(0,10)}</span>
                  <span style={{ fontWeight:700,fontSize:15 }}>Change Request</span>
                  {session && <span style={{ fontSize:12,color:'var(--text-3)' }}>迭代 {session.iteration_count} 轮</span>}
                </div>
                <div style={{ fontSize:12,color:'var(--text-3)',marginTop:2 }}>
                  状态：{cr.status} · {new Date(cr.updated_at).toLocaleString('zh')}
                </div>
              </div>
              <div className="audit-decide">
                {decided
                  ? <span className={'chip '+(decided==='approved'?'green':decided==='rejected'?'red':'amber')} style={{ padding:'7px 14px',fontSize:13 }}>
                      <Icon name={decided==='approved'?'check':decided==='rejected'?'x':'refresh'} size={14} />
                      {decided==='approved'?'已批准 · 合并到 dev':decided==='rejected'?'已拒绝':'已退回 · 重新执行'}
                    </span>
                  : <>
                      <button className="btn btn-danger" onClick={() => doReview('rejected')} disabled={submitting}><Icon name="x" size={15} />拒绝</button>
                      <button className="btn" onClick={() => doReview('revision')} disabled={submitting}><Icon name="refresh" size={15} />修改</button>
                      <button className="btn btn-primary" onClick={() => doReview('approved')} disabled={submitting}><Icon name="check" size={15} />批准合并</button>
                    </>}
              </div>
            </div>
            <div className="audit-split">
              <div className="audit-left scroll">
                <div style={{ padding:'14px 22px 0',display:'flex',alignItems:'center',gap:10 }}>
                  <div className="seg">
                    <button className={tab==='report'?'on':''} onClick={() => setTab('report')}>实现报告</button>
                    <button className={tab==='diff'?'on':''} onClick={() => setTab('diff')}>代码 Diff</button>
                  </div>
                  {tab==='diff' && (
                    <div className="seg" style={{ marginLeft:'auto' }}>
                      <button className={diffMode==='unified'?'on':''} onClick={() => setDiffMode('unified')}><Icon name="rows" size={13} style={{ verticalAlign:-2,marginRight:4 }} />统一</button>
                      <button className={diffMode==='split'?'on':''} onClick={() => setDiffMode('split')}><Icon name="columns" size={13} style={{ verticalAlign:-2,marginRight:4 }} />分栏</button>
                    </div>
                  )}
                </div>
                {tab==='report' ? (
                  <div className="report">
                    {session && (session.iteration_count ?? 0) >= 3 && (
                      <div className="iter-warn"><Icon name="alert" size={20} /><div>已迭代 <b>{session.iteration_count}</b> 轮（软上限 3）。建议手动介入或重新描述需求。</div></div>
                    )}
                    {report ? (
                      <>
                        <h2><Icon name="zap" size={18} style={{ color:'var(--ember)' }} />改动摘要</h2>
                        <p>{report.summary}</p>
                        {report.files.length > 0 && (<><h2><Icon name="file" size={18} style={{ color:'var(--blue)' }} />修改文件</h2>
                          <div>{report.files.map((f, i) => (
                            <span className="file-pill" key={i}><Icon name="file" size={13} />{f.name}<span className="add">+{f.add}</span>{f.del>0&&<span className="del">-{f.del}</span>}</span>
                          ))}</div></>)}
                        {report.testsSection && (<><h2><Icon name="flask" size={18} style={{ color:'var(--green)' }} />测试情况</h2><p style={{ whiteSpace:'pre-line' }}>{report.testsSection}</p></>)}
                        {report.risk && (<><h2><Icon name="shield" size={18} style={{ color:'var(--violet)' }} />潜在风险</h2><p>{report.risk}</p></>)}
                      </>
                    ) : (
                      <div style={{ color:'var(--text-3)',padding:'20px 0' }}>{session ? '报告内容为空' : '加载中…'}</div>
                    )}
                  </div>
                ) : (
                  <div className="diff">
                    {hunks.length === 0
                      ? <div style={{ padding:'20px 22px',color:'var(--text-3)' }}>{diff === '' ? '加载中…' : 'Diff 为空或 worktree 不存在'}</div>
                      : hunks.map((h, hi) => (
                        <div key={hi}>
                          <div className="diff-toolbar" style={{ position:'sticky',top:0 }}>
                            <Icon name="file" size={15} style={{ color:'var(--text-3)' }} />
                            <span className="diff-file">{h.file}</span>
                          </div>
                          <div className="diff-hunk">{h.hunk}</div>
                          {diffMode==='unified'
                            ? h.lines.map((l, i) => (
                              <div key={i} className={'diff-line '+(l.t==='add'?'add':l.t==='del'?'del':'')}>
                                <span className="gut">{l.n1}</span><span className="gut">{l.n2}</span>
                                <span className="code">{l.t==='add'?'+ ':l.t==='del'?'- ':'  '}{l.code}</span>
                              </div>
                            ))
                            : <div className="diff-split-wrap">
                                <div>{h.lines.filter(l=>l.t!=='add').map((l,i)=><div key={i} className={'diff-line '+(l.t==='del'?'del':'')}><span className="gut">{l.n1}</span><span className="code">{l.code}</span></div>)}</div>
                                <div>{h.lines.filter(l=>l.t!=='del').map((l,i)=><div key={i} className={'diff-line '+(l.t==='add'?'add':'')}><span className="gut">{l.n2}</span><span className="code">{l.code}</span></div>)}</div>
                              </div>}
                        </div>
                      ))}
                  </div>
                )}
              </div>
              <div className="audit-right">
                <div className="prev-head"><Icon name="eye" size={16} style={{ color:'var(--ember)' }} /><span style={{ fontWeight:700,fontSize:13.5 }}>预览对比</span></div>
                <div style={{ flex:1,display:'flex',alignItems:'center',justifyContent:'center',color:'var(--text-3)',fontSize:13,padding:18,textAlign:'center' }}>
                  {preview
                    ? <div>
                        <div className={'chip '+(preview.status === 'ready' ? 'green' : 'amber')} style={{ marginBottom:10 }}>{preview.status}</div>
                        <div style={{ fontFamily:'var(--font-mono)',fontSize:11,wordBreak:'break-all',marginBottom:12 }}>{preview.preview_url}</div>
                        {preview.preview_url && <button className="btn btn-primary" onClick={() => window.open(preview.preview_url, '_blank')}><Icon name="eye" size={14} />打开预览</button>}
                      </div>
                    : '暂无预览环境记录'}
                </div>
                <div style={{ padding:'6px 12px 2px',fontFamily:'var(--font-mono)',fontSize:10.5,letterSpacing:'.1em',textTransform:'uppercase',color:'var(--text-faint)' }}>管理员建议 → Claude Code</div>
                <div className="advice-box">
                  <textarea value={advice} onChange={e => setAdvice(e.target.value)} placeholder="在此输入给 Claude Code 的修改意见，点「修改」后进入新一轮迭代…" />
                </div>
              </div>
            </div>
          </>
        ) : (
          <div className="empty" style={{ height:'100%' }}><Icon name="audit" /><div>选择一个待审核需求</div></div>
        )}
      </div>
    </>
  );
}
