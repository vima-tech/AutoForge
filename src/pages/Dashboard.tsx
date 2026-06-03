import React, { useState, useEffect, useCallback } from 'react';
import { listen } from '@tauri-apps/api/event';
import Icon from '../components/Icon';
import { getPipelineStats, listProjects, listIssues, submitIssue, type PipelineStats, type Project, type Issue } from '../services';

const SEV_COLOR: Record<string, string> = {
  critical: 'red', high: 'amber', medium: 'blue', low: 'green',
  Bug: 'red', Feature: 'ember', Improvement: 'blue', Debt: 'violet',
};

// ── Submit Issue Modal ────────────────────────────────────────────────────────
function SubmitIssueModal({ projects, onClose }: { projects: Project[]; onClose: () => void }) {
  const [form, setForm] = useState({ project_id: projects[0]?.id ?? '', title: '', description: '', category: 'Feature', severity: 'medium' });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  const submit = async () => {
    if (!form.title.trim()) { setError('标题不能为空'); return; }
    setLoading(true);
    try {
      await submitIssue(form);
      onClose();
    } catch (e) { setError(String(e)); }
    finally { setLoading(false); }
  };

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 60 }} onClick={onClose}>
      <div style={{ width: 480, background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', overflow: 'hidden' }} onClick={e => e.stopPropagation()}>
        <div style={{ padding: '18px 20px 14px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <div className="eyebrow" style={{ fontSize: 16 }}><span className="cn">提交需求</span></div>
          <button className="icon-btn" onClick={onClose}><Icon name="x" size={18} /></button>
        </div>
        <div style={{ padding: '16px 20px' }}>
          <div className="cfg-fields" style={{ gridTemplateColumns: '1fr 1fr' }}>
            <div className="field full"><label>项目</label>
              <select value={form.project_id} onChange={e => setForm(f => ({ ...f, project_id: e.target.value }))}>
                {projects.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
              </select>
            </div>
            <div className="field full"><label>需求标题</label>
              <input value={form.title} onChange={e => setForm(f => ({ ...f, title: e.target.value }))} placeholder="简洁描述需求" />
            </div>
            <div className="field full"><label>详细描述</label>
              <textarea rows={3} value={form.description} onChange={e => setForm(f => ({ ...f, description: e.target.value }))} placeholder="背景、期望行为、截图说明等" />
            </div>
            <div className="field"><label>分类</label>
              <select value={form.category} onChange={e => setForm(f => ({ ...f, category: e.target.value }))}>
                <option>Feature</option><option>Bug</option><option>Improvement</option><option>Debt</option>
              </select>
            </div>
            <div className="field"><label>严重级别</label>
              <select value={form.severity} onChange={e => setForm(f => ({ ...f, severity: e.target.value }))}>
                <option value="critical">Critical</option><option value="high">High</option>
                <option value="medium">Medium</option><option value="low">Low</option>
              </select>
            </div>
          </div>
          {error && <div style={{ color: 'var(--red)', fontSize: 13, marginTop: 10 }}>{error}</div>}
        </div>
        <div style={{ padding: '14px 20px', borderTop: '1px solid var(--border)', display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn btn-primary" onClick={submit} disabled={loading}>
            <Icon name="send" size={15} />{loading ? '提交中…' : '提交需求'}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Dashboard ─────────────────────────────────────────────────────────────────
export default function Dashboard() {
  const [stats, setStats] = useState<PipelineStats | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [issues, setIssues] = useState<Issue[]>([]);
  const [showSubmit, setShowSubmit] = useState(false);

  const loadAll = useCallback(async () => {
    const [s, ps, is] = await Promise.all([getPipelineStats(), listProjects(), listIssues()]);
    setStats(s);
    setProjects(ps);
    setIssues(is.sort((a, b) => (b.priority ?? 0) - (a.priority ?? 0)).slice(0, 5));
  }, []);

  useEffect(() => {
    loadAll();
    let unlisten: (() => void) | undefined;
    listen<unknown>('autoforge://event', () => loadAll()).then(fn => { unlisten = fn; });
    return () => unlisten?.();
  }, [loadAll]);

  const pipeline = [
    { ic: 'inbox', name: '需求入口',   cnt: stats?.pending_analysis ?? 0, state: 'done' },
    { ic: 'search', name: '需求分析',  cnt: stats?.pending_review_1 ?? 0, state: 'done' },
    { ic: 'check', name: '审核 1',     cnt: stats?.pending_review_1 ?? 0, state: 'active' },
    { ic: 'code', name: 'Claude Code', cnt: stats?.executing ?? 0,        state: 'active' },
    { ic: 'eye', name: '审核 2',       cnt: stats?.pending_review_2 ?? 0, state: stats?.pending_review_2 ?? 0 > 3 ? 'warn' : 'active' },
    { ic: 'merge', name: '合并 dev',   cnt: stats?.merged ?? 0,           state: '' },
  ];

  // Build backlog map
  const backlogByProject: Record<string, number> = {};
  issues.forEach(i => { backlogByProject[i.project_id] = (backlogByProject[i.project_id] ?? 0) + 1; });

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
              {stats ? `${stats.stage === 'normal' ? '流水线运行正常' : stats.stage === 'throttled' ? '单线程降速中' : '系统已暂停'} · ${stats.active_slots}/${stats.max_slots} 槽位占用` : '加载中…'}
            </div>
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <button className="btn btn-primary" onClick={() => setShowSubmit(true)}><Icon name="plus" size={16} />提交需求</button>
          </div>
        </div>

        {/* stats */}
        <div className="stat-grid">
          {[
            { ic: 'box',   color: '#e8772e', val: String(stats?.active_projects ?? '—'), unit: '', label: '在产项目', delta: '' },
            { ic: 'inbox', color: '#8b7ad8', val: String(stats?.total_issues ?? '—'), unit: '', label: '需求总数', delta: '' },
            { ic: 'cpu',   color: '#4f8ed1', val: String(stats?.active_slots ?? '—'), unit: `/${stats?.max_slots ?? 5}`, label: '并发槽位占用', delta: `阶段 · ${stats?.stage ?? '…'}` },
            { ic: 'clock', color: '#4f9d6b', val: String(stats?.pending_review_2 ?? '—'), unit: '', label: '待审核 (审核2)', delta: '' },
          ].map((s, i) => (
            <div className="stat" key={i}>
              <div className="stat-ic" style={{ background: `color-mix(in oklab, ${s.color} 16%, transparent)`, color: s.color }}><Icon name={s.ic} size={18} /></div>
              <div className="stat-val">{s.val}<span className="u">{s.unit}</span></div>
              <div className="stat-label">{s.label}</div>
              {s.delta && <div className="stat-delta up">{s.delta}</div>}
            </div>
          ))}
        </div>

        {/* pipeline */}
        <div className="panel" style={{ marginBottom: 16 }}>
          <div className="panel-head">
            <div className="eyebrow" style={{ fontSize: 14 }}><span className="en">PIPELINE</span><span className="cn">· 完整流水线</span></div>
            <div className="sec-kicker">实时 · LIVE</div>
          </div>
          <div className="pipe scroll">
            {pipeline.map((p, i) => (
              <div className="pipe-stage" key={i}>
                <div className={'pipe-node ' + p.state}><Icon name={p.ic} size={20} /></div>
                <div className="pipe-cnt" style={{ color: p.state==='active'?'var(--ember)':p.state==='warn'?'var(--amber)':p.state==='done'?'var(--green-soft)':'var(--text-2)' }}>{p.cnt}</div>
                <div className="pipe-name">{p.name}</div>
              </div>
            ))}
          </div>
        </div>

        <div className="dash-cols">
          {/* queue */}
          <div className="panel">
            <div className="panel-head">
              <div className="panel-title"><Icon name="inbox" size={17} style={{ color: 'var(--ember)' }} />需求队列 · 优先级排序</div>
              <span className="sec-kicker">共 {stats?.total_issues ?? 0} 条</span>
            </div>
            {issues.length === 0
              ? <div style={{ padding: '20px 18px', color: 'var(--text-3)', fontSize: 13 }}>暂无需求</div>
              : issues.map((q, i) => (
              <div className="q-row" key={q.id}>
                <div className="q-pr">{i + 1}</div>
                <div className="q-main">
                  <div className="q-title">{q.title}</div>
                  <div className="q-meta">
                    <span className="req-id" style={{ color: 'var(--text-3)' }}>{q.id.slice(0, 10)}</span>
                    <span className={'chip ' + (SEV_COLOR[q.category] || 'blue')} style={{ padding: '1px 7px', fontSize: 10 }}>{q.category}</span>
                    <span>· {projects.find(p => p.id === q.project_id)?.name ?? '—'}</span>
                  </div>
                </div>
                <span className={'chip ' + (q.status.includes('review') ? 'amber' : q.status === 'executing' ? 'ember' : '')}>{q.status.replace(/_/g,' ')}</span>
              </div>
            ))}
          </div>

          {/* slots + backpressure */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
            <div className="panel">
              <div className="panel-head">
                <div className="panel-title"><Icon name="cpu" size={17} style={{ color: 'var(--blue)' }} />并发槽位</div>
                <span className="sec-kicker">{stats?.active_slots ?? 0} / {stats?.max_slots ?? 5}</span>
              </div>
              <div style={{ padding: '14px 18px 18px' }}>
                <div className="slots">
                  {Array.from({ length: stats?.max_slots ?? 5 }).map((_, i) => {
                    const crId = stats?.executing_cr_ids[i];
                    return <div key={i} className={'slot' + (crId ? ' busy' : '')}>{crId ? crId.slice(0, 10) : '空闲'}</div>;
                  })}
                </div>
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
                  <div className="bp-seg" style={{ width: `${Math.min(100, ((stats?.pending_review_slots ?? 0) / (stats?.pause_threshold ?? 20)) * 100)}%`, background: 'var(--green)' }} />
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between', fontFamily: 'var(--font-mono)', fontSize: 10.5, color: 'var(--text-faint)' }}>
                  <span>积压 0</span><span>暂停 {stats?.pause_threshold ?? 20}</span>
                </div>
              </div>
            </div>
          </div>
        </div>

        {/* projects */}
        <div className="panel">
          <div className="panel-head">
            <div className="eyebrow" style={{ fontSize: 14 }}><span className="en">PROJECTS</span><span className="cn">· 在产项目</span></div>
          </div>
          {projects.length === 0
            ? <div style={{ padding: '20px 18px', color: 'var(--text-3)', fontSize: 13 }}>暂无项目，点击「接入项目」添加</div>
            : <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 1, background: 'var(--border)' }}>
              {projects.map(p => (
                <div key={p.id} style={{ display: 'flex', alignItems: 'center', gap: 13, padding: '15px 18px', background: 'var(--bg-2)' }}>
                  <div className="proj-logo" style={{ background: '#e8772e', width: 42, height: 42, fontSize: 18 }}>{p.name[0]}</div>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                      <span style={{ fontWeight: 700, fontSize: 14 }}>{p.name}</span>
                      <span className={'dot ' + (p.status === 'active' ? 'green' : 'gray')} />
                    </div>
                    <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 2 }}>{p.description}</div>
                  </div>
                  <div style={{ textAlign: 'right' }}>
                    <div style={{ fontFamily: 'var(--font-display)', fontWeight: 800, fontSize: 20 }}>
                      {issues.filter(i => i.project_id === p.id).length}
                    </div>
                    <div style={{ fontSize: 10.5, color: 'var(--text-faint)', fontFamily: 'var(--font-mono)' }}>BACKLOG</div>
                  </div>
                </div>
              ))}
            </div>}
        </div>
      </div>
    </div>
  );
}
