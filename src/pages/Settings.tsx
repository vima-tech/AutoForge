import React, { useState, useEffect } from 'react';
import Icon from '../components/Icon';
import { Avatar } from '../components/Avatar';
import {
  listLlmConfigs, createLlmConfig, updateLlmConfig, deleteLlmConfig, testLlmConnection,
  listAgents, createAgent, updateAgent, deleteAgent,
  getSystemHealth, updateConcurrencyConfig, readSpec, writeSpec,
  listPreviewEnvironments, listTestSessions, listAdminDecisions,
  type LlmConfig, type Agent, type SystemHealth, type PreviewEnvironment,
  type TestSession, type AdminDecision,
} from '../services';

// ── helpers ──────────────────────────────────────────────────────────────────
function Switch({ on, onToggle }: { on: boolean; onToggle: () => void }) {
  return <button className={'switch' + (on ? ' on' : '')} onClick={onToggle}><i /></button>;
}

function ConfirmModal({ msg, onOk, onCancel }: { msg: string; onOk: () => void; onCancel: () => void }) {
  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.5)', display: 'grid', placeItems: 'center', zIndex: 9999 }} onClick={onCancel}>
      <div style={{ background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 14, padding: '22px 24px', width: 360, boxShadow: 'var(--shadow-lg)' }} onClick={e => e.stopPropagation()}>
        <p style={{ margin: '0 0 20px', fontSize: 14, lineHeight: 1.6 }}>{msg}</p>
        <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onCancel}>取消</button>
          <button className="btn btn-danger" onClick={onOk}>确认删除</button>
        </div>
      </div>
    </div>
  );
}

const llmColor = (provider: string) => {
  const p = provider.toLowerCase();
  if (p.includes('anthropic')) return '#8b7ad8';
  if (p.includes('openai')) return '#4f8ed1';
  if (p.includes('ollama')) return '#4f9d6b';
  return '#e8772e';
};

// ── LLM Settings ─────────────────────────────────────────────────────────────
function LLMSettings() {
  const [configs, setConfigs] = useState<LlmConfig[]>([]);
  const [exp, setExp] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, Partial<LlmConfig>>>({});
  const [testing, setTesting] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<Record<string, string>>({});
  const [confirmDel, setConfirmDel] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    listLlmConfigs().then(cs => { setConfigs(cs); setLoading(false); }).catch(() => setLoading(false));
  }, []);

  const setDraft = (id: string, field: string, val: unknown) =>
    setDrafts(d => ({ ...d, [id]: { ...d[id], [field]: val } }));

  const save = async (id: string) => {
    const d = drafts[id] ?? {};
    if (Object.keys(d).length === 0) return;
    const updated = await updateLlmConfig(id, d);
    setConfigs(cs => cs.map(c => c.id === id ? updated : c));
    setDrafts(d2 => { const n = { ...d2 }; delete n[id]; return n; });
  };

  const toggleEnabled = async (id: string, cur: boolean) => {
    const updated = await updateLlmConfig(id, { enabled: !cur });
    setConfigs(cs => cs.map(c => c.id === id ? updated : c));
  };

  const testConn = async (id: string) => {
    setTesting(id);
    const result = await testLlmConnection(id).catch(e => String(e));
    setTestResult(r => ({ ...r, [id]: result }));
    setTesting(null);
  };

  const doDelete = async (id: string) => {
    await deleteLlmConfig(id);
    setConfigs(cs => cs.filter(c => c.id !== id));
    setConfirmDel(null);
  };

  const addNew = async () => {
    const c = await createLlmConfig({
      name: '新 LLM 配置', provider: 'Anthropic',
      model: 'claude-sonnet-4-20250514', endpoint: 'https://api.anthropic.com', api_key: '',
    });
    setConfigs(cs => [...cs, c]);
    setExp(c.id);
  };

  if (loading) return <div className="set-inner"><div className="set-h">LLM 配置</div><div style={{ color: 'var(--text-3)', marginTop: 20 }}>加载中…</div></div>;

  return (
    <div className="set-inner rise">
      {confirmDel && <ConfirmModal msg="确认删除此 LLM 配置？" onOk={() => doDelete(confirmDel)} onCancel={() => setConfirmDel(null)} />}
      <div className="set-h">LLM 配置</div>
      <div className="set-desc">管理多个大模型连接。每个 Agent 可指派不同的 LLM。</div>
      {configs.map(c => {
        const d = drafts[c.id] ?? {};
        const v = (f: keyof LlmConfig) => (d as Record<string, unknown>)[f] !== undefined ? (d as Record<string, unknown>)[f] as string : c[f] as string;
        return (
          <div className="cfg-card" key={c.id} style={exp === c.id ? { borderColor: 'var(--ember-tint-strong)' } : {}}>
            <div className="cfg-top" onClick={() => setExp(exp === c.id ? null : c.id)} style={{ cursor: 'pointer' }}>
              <div className="cfg-logo" style={{ background: llmColor(String(v('provider'))) }}><Icon name="brain" size={20} /></div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div className="cfg-name">{v('name')}</div>
                <div className="cfg-sub">{v('provider')} · {v('model')}</div>
              </div>
              <span className={'chip ' + (c.enabled ? 'green' : '')}>{c.enabled ? '● 已启用' : '未启用'}</span>
              <Icon name={exp === c.id ? 'chevDown' : 'chevRight'} size={18} style={{ color: 'var(--text-3)', marginLeft: 4 }} />
            </div>
            {exp === c.id && (
              <div className="cfg-fields rise">
                <div className="field"><label>Provider</label>
                  <select value={v('provider')} onChange={e => setDraft(c.id, 'provider', e.target.value)}>
                    <option>Anthropic</option><option>OpenAI</option><option>Ollama</option><option>Azure</option><option>自定义</option>
                  </select>
                </div>
                <div className="field"><label>Model</label><input className="mono" value={v('model')} onChange={e => setDraft(c.id, 'model', e.target.value)} /></div>
                <div className="field full"><label>API Endpoint</label><input className="mono" value={v('endpoint')} onChange={e => setDraft(c.id, 'endpoint', e.target.value)} /></div>
                <div className="field full"><label><Icon name="key" size={11} style={{ verticalAlign: -1, marginRight: 4 }} />API Key</label>
                  <input className="mono" value={v('api_key')} onChange={e => setDraft(c.id, 'api_key', e.target.value)} type="password" />
                </div>
                <div className="field"><label>上下文窗口</label><input value={v('ctx_window')} onChange={e => setDraft(c.id, 'ctx_window', e.target.value)} /></div>
                <div className="field"><label>Temperature</label>
                  <input type="number" step="0.1" min="0" max="2"
                    value={d.temperature !== undefined ? String(d.temperature) : String(c.temperature)}
                    onChange={e => setDraft(c.id, 'temperature', parseFloat(e.target.value))} />
                </div>
                <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 12, marginTop: 4 }}>
                  <Switch on={c.enabled} onToggle={() => toggleEnabled(c.id, c.enabled)} />
                  <span style={{ fontSize: 13, color: 'var(--text-2)' }}>启用此连接</span>
                  {testResult[c.id] && <span style={{ fontSize: 12, color: testResult[c.id].startsWith('连接成功') ? 'var(--green-soft)' : 'var(--red)', fontFamily: 'var(--font-mono)' }}>{testResult[c.id]}</span>}
                  <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
                    <button className="btn btn-sm btn-danger" onClick={() => setConfirmDel(c.id)}><Icon name="trash" size={13} />删除</button>
                    <button className="btn btn-sm" onClick={() => testConn(c.id)} disabled={testing === c.id}>
                      <Icon name="zap" size={13} />{testing === c.id ? '测试中…' : '测试连接'}
                    </button>
                    <button className="btn btn-sm btn-primary" onClick={() => save(c.id)} disabled={Object.keys(d).length === 0}>
                      <Icon name="check" size={13} />保存
                    </button>
                  </div>
                </div>
              </div>
            )}
          </div>
        );
      })}
      <div className="cfg-card add" onClick={addNew}><Icon name="plus" size={18} />添加 LLM 配置</div>
    </div>
  );
}

// ── Agent Settings ────────────────────────────────────────────────────────────
function AgentSettings() {
  const [agents, setAgents] = useState<Agent[]>([]);
  const [llmNames, setLlmNames] = useState<{ id: string; name: string }[]>([]);
  const [exp, setExp] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, Partial<Agent>>>({});
  const [confirmDel, setConfirmDel] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    Promise.all([listAgents(), listLlmConfigs()]).then(([ags, llms]) => {
      setAgents(ags);
      setLlmNames(llms.map(l => ({ id: l.id, name: l.name })));
      setLoading(false);
    }).catch(() => setLoading(false));
  }, []);

  const setDraft = (id: string, field: string, val: unknown) =>
    setDrafts(d => ({ ...d, [id]: { ...d[id], [field]: val } }));

  const save = async (id: string) => {
    const d = drafts[id] ?? {};
    if (Object.keys(d).length === 0) return;
    const updated = await updateAgent(id, d);
    setAgents(as => as.map(a => a.id === id ? updated : a));
    setDrafts(d2 => { const n = { ...d2 }; delete n[id]; return n; });
  };

  const setForgeRole = async (id: string, role: 'analysis' | 'test' | null) => {
    const updated = await updateAgent(id, { forge_role: role });
    setAgents(as => as.map(a => a.id === id ? updated : a));
  };

  const doDelete = async (id: string) => {
    await deleteAgent(id);
    setAgents(as => as.filter(a => a.id !== id));
    setConfirmDel(null);
  };

  const addNew = async () => {
    const a = await createAgent({ name: '新 Agent', system_prompt: '' });
    setAgents(as => [...as, a]);
    setExp(a.id);
  };

  if (loading) return <div className="set-inner"><div className="set-h">Agent 配置</div><div style={{ color: 'var(--text-3)', marginTop: 20 }}>加载中…</div></div>;

  const analystId = agents.find(a => a.forge_role === 'analysis')?.id;
  const testerId  = agents.find(a => a.forge_role === 'test')?.id;

  return (
    <div className="set-inner rise">
      {confirmDel && <ConfirmModal msg="确认删除此 Agent？" onOk={() => doDelete(confirmDel)} onCancel={() => setConfirmDel(null)} />}
      <div className="set-h">Agent 配置</div>
      <div className="set-desc">配置 Agent 职能、LLM、系统提示词，并指派流水线角色。</div>

      <div className="panel" style={{ marginBottom: 22 }}>
        <div className="panel-head"><div className="panel-title"><Icon name="sliders" size={16} style={{ color: 'var(--ember)' }} />流水线角色指派</div></div>
        <div style={{ padding: '4px 18px 14px' }}>
          {[{ role: 'analysis' as const, label: '需求分析 Agent', desc: '在审核节点 1 前评估真实性、可行性、优先级', color: 'var(--violet)', icon: 'search' },
            { role: 'test' as const,     label: '测试 Agent',    desc: '合并后被动响应 + 每日主动巡检',           color: 'var(--green)', icon: 'flask' }
          ].map(({ role, label, desc, color, icon }) => (
            <div key={role} className="assign-row">
              <div className="cfg-logo" style={{ background: color, width: 34, height: 34 }}><Icon name={icon} size={17} /></div>
              <div className="assign-info"><div className="assign-title">{label}</div><div className="assign-desc">{desc}</div></div>
              <select style={{ width: 180, background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 9, padding: '8px 10px', color: 'var(--text)', fontSize: 13 }}
                value={role === 'analysis' ? (analystId ?? '') : (testerId ?? '')}
                onChange={e => { if (e.target.value) setForgeRole(e.target.value, role); }}>
                <option value="">— 未指派 —</option>
                {agents.map(a => <option key={a.id} value={a.id}>{a.name}</option>)}
              </select>
            </div>
          ))}
        </div>
      </div>

      <div className="sec-kicker" style={{ marginBottom: 12 }}>全部 Agent · {agents.length}</div>
      {agents.map(a => {
        const d = drafts[a.id] ?? {};
        const v = (f: keyof Agent) => (d as Record<string, unknown>)[f] !== undefined ? String((d as Record<string, unknown>)[f]) : String(a[f] ?? '');
        const roles = [a.forge_role === 'analysis' && '需求分析', a.forge_role === 'test' && '测试'].filter(Boolean);
        return (
          <div className="cfg-card" key={a.id} style={exp === a.id ? { borderColor: 'var(--ember-tint-strong)' } : {}}>
            <div className="cfg-top" onClick={() => setExp(exp === a.id ? null : a.id)} style={{ cursor: 'pointer' }}>
              <Avatar agent={a} size={40} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div className="cfg-name" style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  {v('name')}
                  {roles.map(r => <span key={r as string} className="chip ember" style={{ padding: '1px 7px', fontSize: 10 }}>{r} Agent</span>)}
                </div>
                <div className="cfg-sub">{v('name_en')} · {llmNames.find(l => l.id === a.llm_id)?.name ?? '未指定 LLM'}</div>
              </div>
              <Icon name={exp === a.id ? 'chevDown' : 'chevRight'} size={18} style={{ color: 'var(--text-3)' }} />
            </div>
            {exp === a.id && (
              <div className="rise" style={{ marginTop: 15 }}>
                <div className="cfg-fields">
                  <div className="field"><label>Agent 名称</label><input value={v('name')} onChange={e => setDraft(a.id, 'name', e.target.value)} /></div>
                  <div className="field"><label>使用的 LLM</label>
                    <select value={d.llm_id !== undefined ? String(d.llm_id ?? '') : (a.llm_id ?? '')}
                      onChange={e => setDraft(a.id, 'llm_id', e.target.value || null)}>
                      <option value="">— 未指定 —</option>
                      {llmNames.map(l => <option key={l.id} value={l.id}>{l.name}</option>)}
                    </select>
                  </div>
                  <div className="field full"><label>职责标签</label><input value={v('role')} onChange={e => setDraft(a.id, 'role', e.target.value)} /></div>
                  <div className="field full"><label>系统提示词</label>
                    <textarea className="mono" rows={5} value={v('system_prompt')} onChange={e => setDraft(a.id, 'system_prompt', e.target.value)} />
                  </div>
                </div>
                <div style={{ display: 'flex', gap: 8, marginTop: 14, paddingTop: 14, borderTop: '1px solid var(--border)', justifyContent: 'flex-end' }}>
                  <button className="btn btn-sm btn-danger" onClick={() => setConfirmDel(a.id)}><Icon name="trash" size={13} />删除</button>
                  <button className="btn btn-sm btn-primary" onClick={() => save(a.id)} disabled={Object.keys(d).length === 0}>
                    <Icon name="check" size={13} />保存
                  </button>
                </div>
              </div>
            )}
          </div>
        );
      })}
      <div className="cfg-card add" onClick={addNew}><Icon name="plus" size={18} />添加 Agent</div>
    </div>
  );
}

function ConcurrencySettings() {
  const [form, setForm] = useState({ max_slots: 5, pause_threshold: 20, queue_strategy: 'priority' });
  const [result, setResult] = useState('');

  const save = async () => {
    const cfg = await updateConcurrencyConfig({
      max_slots: form.max_slots,
      pause_threshold: form.pause_threshold,
      queue_strategy: form.queue_strategy,
    });
    setForm({ max_slots: cfg.max_slots, pause_threshold: cfg.pause_threshold, queue_strategy: cfg.queue_strategy });
    setResult(`${cfg.stage} · ${cfg.active_slots}/${cfg.max_slots} · 待审核 ${cfg.pending_review}`);
  };

  return (
    <div className="set-inner rise">
      <div className="set-h">并发与流控</div>
      <div className="set-desc">控制 Claude Code 执行槽位和审核积压背压阈值。</div>
      <div className="cfg-card">
        <div className="cfg-fields">
          <div className="field"><label>最大并发槽位</label>
            <input type="number" min="1" max="32" value={form.max_slots} onChange={e => setForm(f => ({ ...f, max_slots: Number(e.target.value) }))} />
          </div>
          <div className="field"><label>暂停阈值</label>
            <input type="number" min="1" max="200" value={form.pause_threshold} onChange={e => setForm(f => ({ ...f, pause_threshold: Number(e.target.value) }))} />
          </div>
          <div className="field"><label>队列策略</label>
            <select value={form.queue_strategy} onChange={e => setForm(f => ({ ...f, queue_strategy: e.target.value }))}>
              <option value="priority">priority</option>
              <option value="fifo">fifo</option>
              <option value="oldest">oldest</option>
            </select>
          </div>
          <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
            <button className="btn btn-primary" onClick={save}><Icon name="check" size={14} />保存配置</button>
            {result && <span style={{ fontSize: 12, color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{result}</span>}
          </div>
        </div>
      </div>
    </div>
  );
}

const SPEC_FILES = ['analysis-spec.md', 'coding-spec.md', 'review-spec.md', 'testing-spec.md'];

function SpecsSettings() {
  const [name, setName] = useState(SPEC_FILES[0]);
  const [content, setContent] = useState('');
  const [status, setStatus] = useState('');

  useEffect(() => {
    readSpec(name).then(doc => { setContent(doc.content); setStatus(''); }).catch(e => setStatus(String(e)));
  }, [name]);

  const save = async () => {
    const doc = await writeSpec(name, content);
    setContent(doc.content);
    setStatus('已保存');
  };

  return (
    <div className="set-inner rise">
      <div className="set-h">规范文档</div>
      <div className="set-desc">这些文档会注入 Agent prompt，直接约束分析、编码和测试行为。</div>
      <div className="cfg-card">
        <div className="cfg-fields">
          <div className="field full"><label>文档</label>
            <select value={name} onChange={e => setName(e.target.value)}>
              {SPEC_FILES.map(f => <option key={f} value={f}>{f}</option>)}
            </select>
          </div>
          <div className="field full"><label>内容</label>
            <textarea className="mono" rows={18} value={content} onChange={e => setContent(e.target.value)} />
          </div>
          <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
            <button className="btn btn-primary" onClick={save}><Icon name="check" size={14} />保存文档</button>
            {status && <span style={{ fontSize: 12, color: status === '已保存' ? 'var(--green-soft)' : 'var(--red)' }}>{status}</span>}
          </div>
        </div>
      </div>
    </div>
  );
}

function SecuritySettings() {
  const [decisions, setDecisions] = useState<AdminDecision[]>([]);

  useEffect(() => {
    listAdminDecisions().then(setDecisions).catch(() => setDecisions([]));
  }, []);

  return (
    <div className="set-inner rise">
      <div className="set-h">安全与权限</div>
      <div className="set-desc">当前启用输入消毒、Git 代理拦截、合并唯一入口和管理员决策审计。</div>
      <div className="panel" style={{ marginBottom: 16 }}>
        <div className="panel-head"><div className="panel-title"><Icon name="shield" size={16} style={{ color: 'var(--green)' }} />已启用防护</div></div>
        <div style={{ padding: '12px 18px', display: 'grid', gap: 8, fontSize: 13, color: 'var(--text-2)' }}>
          <div>输入消毒：拦截明显 prompt injection 片段</div>
          <div>Git 代理：阻止 push main/master、force push、remote set-url、global config</div>
          <div>合并入口：只有审核 2 批准后可入队 merge</div>
          <div>审计链：review_1 / review_2 决策写入 admin_decisions</div>
        </div>
      </div>
      <div className="sec-kicker" style={{ marginBottom: 10 }}>最近决策 · {decisions.length}</div>
      {decisions.slice(0, 20).map(d => (
        <div className="cfg-card" key={d.id}>
          <div className="cfg-top">
            <div className="cfg-logo" style={{ background: d.decision === 'approved' ? 'var(--green)' : d.decision === 'rejected' ? 'var(--red)' : 'var(--amber)' }}>
              <Icon name={d.decision === 'approved' ? 'check' : d.decision === 'rejected' ? 'x' : 'refresh'} size={18} />
            </div>
            <div style={{ flex: 1 }}>
              <div className="cfg-name">{d.stage} · {d.decision}</div>
              <div className="cfg-sub">{d.issue_id.slice(0, 10)} · {new Date(d.created_at).toLocaleString('zh')}</div>
            </div>
          </div>
          {d.suggestions && <div style={{ padding: '0 2px 2px', fontSize: 13, color: 'var(--text-3)' }}>{d.suggestions}</div>}
        </div>
      ))}
      {decisions.length === 0 && <div style={{ color: 'var(--text-3)', fontSize: 13 }}>暂无审计记录</div>}
    </div>
  );
}

function AboutSettings() {
  const [health, setHealth] = useState<SystemHealth | null>(null);
  const [previews, setPreviews] = useState<PreviewEnvironment[]>([]);
  const [tests, setTests] = useState<TestSession[]>([]);

  useEffect(() => {
    getSystemHealth().then(setHealth).catch(() => setHealth(null));
    listPreviewEnvironments().then(setPreviews).catch(() => setPreviews([]));
    listTestSessions().then(setTests).catch(() => setTests([]));
  }, []);

  return (
    <div className="set-inner rise">
      <div className="set-h">关于 AutoForge</div>
      <div className="set-desc">运行健康、Claude 认证和后台运行态概览。</div>
      <div className="stat-grid" style={{ gridTemplateColumns: 'repeat(4, minmax(0, 1fr))', marginBottom: 16 }}>
        {[
          { label: '数据库', val: health?.db_ok ? 'OK' : '—', color: 'var(--green)' },
          { label: 'Claude Auth', val: health?.claude_auth ? 'OK' : '未登录', color: health?.claude_auth ? 'var(--green)' : 'var(--red)' },
          { label: '版本', val: health?.version ?? '—', color: 'var(--blue)' },
          { label: '阶段', val: health?.stage ?? '—', color: 'var(--ember)' },
        ].map(x => (
          <div className="stat" key={x.label}>
            <div className="stat-val" style={{ color: x.color }}>{x.val}</div>
            <div className="stat-label">{x.label}</div>
          </div>
        ))}
      </div>
      <div className="panel" style={{ marginBottom: 16 }}>
        <div className="panel-head"><div className="panel-title"><Icon name="eye" size={16} style={{ color: 'var(--ember)' }} />预览环境</div><span className="sec-kicker">{previews.length}</span></div>
        <div style={{ padding: '8px 18px 14px', display: 'grid', gap: 8 }}>
          {previews.slice(0, 8).map(p => <div key={p.id} style={{ fontSize: 12, color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{p.status} · {p.preview_url || p.id}</div>)}
          {previews.length === 0 && <div style={{ fontSize: 13, color: 'var(--text-3)' }}>暂无预览环境</div>}
        </div>
      </div>
      <div className="panel">
        <div className="panel-head"><div className="panel-title"><Icon name="flask" size={16} style={{ color: 'var(--green)' }} />测试会话</div><span className="sec-kicker">{tests.length}</span></div>
        <div style={{ padding: '8px 18px 14px', display: 'grid', gap: 8 }}>
          {tests.slice(0, 8).map(t => <div key={t.id} style={{ fontSize: 12, color: 'var(--text-3)' }}>{t.status} · {t.summary || t.id}</div>)}
          {tests.length === 0 && <div style={{ fontSize: 13, color: 'var(--text-3)' }}>暂无测试会话</div>}
        </div>
      </div>
    </div>
  );
}

const SET_ITEMS = [
  { id: 'llm',         name: 'LLM 配置',     ic: 'brain' },
  { id: 'agents',      name: 'Agent 配置',   ic: 'bot' },
  { id: 'concurrency', name: '并发与流控',   ic: 'cpu' },
  { id: 'security',    name: '安全与权限',   ic: 'shield' },
  { id: 'specs',       name: '规范文档',     ic: 'file' },
  { id: 'about',       name: '关于 AutoForge', ic: 'zap' },
];

export default function SettingsPage() {
  const [sec, setSec] = useState('llm');
  const cur = SET_ITEMS.find(i => i.id === sec)!;
  return (
    <div className="content">
      <div className="audit-top" style={{ height: 56 }}>
        <div className="eyebrow" style={{ fontSize: 17 }}><span className="en">SETTINGS</span><span className="cn">· 设置</span></div>
      </div>
      <div className="set-wrap">
        <div className="set-nav">
          {SET_ITEMS.map(it => (
            <div key={it.id} className={'set-nav-item' + (sec === it.id ? ' active' : '')} onClick={() => setSec(it.id)}>
              <Icon name={it.ic} size={18} />{it.name}
            </div>
          ))}
        </div>
        <div className="set-body scroll">
          {sec === 'llm'    && <LLMSettings />}
          {sec === 'agents' && <AgentSettings />}
          {sec === 'concurrency' && <ConcurrencySettings />}
          {sec === 'security' && <SecuritySettings />}
          {sec === 'specs' && <SpecsSettings />}
          {sec === 'about' && <AboutSettings />}
          {!['llm','agents','concurrency','security','specs','about'].includes(sec) && (
            <div className="empty" style={{ height: '100%' }}>
              <Icon name={cur.ic} /><div>{cur.name}</div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
