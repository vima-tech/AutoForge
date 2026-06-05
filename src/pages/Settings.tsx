import React, { useState, useEffect } from 'react';
import { createPortal } from 'react-dom';
import Icon from '../components/Icon';
import { Avatar } from '../components/Avatar';
import Select from '../components/Select';
import { THEME_PALETTES, type ThemeMode, type ThemeSelection } from '../theme';
import {
  listLlmConfigs, createLlmConfig, updateLlmConfig, deleteLlmConfig, testLlmConnection,
  listAgents, createAgent, updateAgent, deleteAgent, setAgentForgeRole,
  getSystemHealth, updateConcurrencyConfig, readSpec, writeSpec,
  listPreviewEnvironments, listTestSessions, listAdminDecisions,
  getIntakeConfig, updateIntakeConfig, getWebhookStatus,
  type LlmConfig, type Agent, type SystemHealth, type PreviewEnvironment,
  type TestSession, type AdminDecision, type IntakeConfig, type WebhookStatus,
} from '../services';

// ── helpers ──────────────────────────────────────────────────────────────────
function Switch({ on, onToggle }: { on: boolean; onToggle: () => void }) {
  return <button className={'switch' + (on ? ' on' : '')} onClick={onToggle}><i /></button>;
}

function ConfirmModal({ msg, onOk, onCancel }: { msg: string; onOk: () => void; onCancel: () => void }) {
  return createPortal(
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.5)', display: 'grid', placeItems: 'center', zIndex: 9999 }} onClick={onCancel}>
      <div style={{ background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 14, padding: '22px 24px', width: 360, boxShadow: 'var(--shadow-lg)' }} onClick={e => e.stopPropagation()}>
        <p style={{ margin: '0 0 20px', fontSize: 'var(--text-body)', lineHeight: 'var(--leading-relaxed)' }}>{msg}</p>
        <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onCancel}>取消</button>
          <button className="btn btn-danger" onClick={onOk}>确认删除</button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

const llmColor = (provider: string) => {
  const p = provider.toLowerCase();
  if (p.includes('anthropic')) return '#8b7ad8';
  if (p.includes('openai')) return '#4f8ed1';
  if (p.includes('ollama')) return '#4f9d6b';
  return '#e8772e';
};

function formatAgentSub(agent: Agent, llmNames: { id: string; name: string }[]) {
  const llmName = llmNames.find(l => l.id === agent.llm_id)?.name ?? '未指定 LLM';
  return `LLM: ${llmName}`;
}

// 从后端返回的完整错误字符串中提取简短摘要，保留 HTTP 状态码 + 核心 message
function fmtTestResult(raw: string): string {
  if (raw.startsWith('连接成功')) return raw;
  const jsonIdx = raw.indexOf('{');
  if (jsonIdx === -1) return raw;
  const prefix = raw.slice(0, jsonIdx).replace(/·\s*$/, '').trim();
  try {
    const body = JSON.parse(raw.slice(jsonIdx));
    const msg: string = body?.error?.message ?? body?.message ?? '';
    return msg ? `${prefix} · ${msg}` : prefix;
  } catch {
    return raw.slice(0, 80);
  }
}

// ── LLM Settings ─────────────────────────────────────────────────────────────
function LLMSettings() {
  const [configs, setConfigs] = useState<LlmConfig[]>([]);
  const [exp, setExp] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, Partial<LlmConfig>>>({});
  const [testing, setTesting] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<Record<string, string>>({});
  const [saveStatus, setSaveStatus] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState<string | null>(null);
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
    setSaving(id);
    try {
      const updated = await updateLlmConfig(id, d);
      setConfigs(cs => cs.map(c => c.id === id ? updated : c));
      setDrafts(d2 => { const n = { ...d2 }; delete n[id]; return n; });
      setSaveStatus(s => ({ ...s, [id]: '已保存' }));
      setTimeout(() => setSaveStatus(s => { const n = { ...s }; delete n[id]; return n; }), 2500);
    } catch (e) {
      setSaveStatus(s => ({ ...s, [id]: '保存失败: ' + String(e) }));
    } finally {
      setSaving(null);
    }
  };

  const toggleEnabled = async (id: string, cur: boolean) => {
    const updated = await updateLlmConfig(id, { enabled: !cur });
    setConfigs(cs => cs.map(c => c.id === id ? updated : c));
  };

  const testConn = async (id: string) => {
    setTesting(id);
    setTestResult(r => { const n = { ...r }; delete n[id]; return n; });
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
                <div className="field full"><label>名称</label><input value={v('name')} onChange={e => setDraft(c.id, 'name', e.target.value)} /></div>
                <div className="field"><label>Provider</label>
                  <Select value={v('provider')} onChange={val => setDraft(c.id, 'provider', val)}
                    options={['Anthropic', 'OpenAI', 'Ollama', 'Azure', '自定义'].map(v => ({ value: v, label: v }))} />
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
                <div className="field full" style={{ gap: 10, marginTop: 4 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                    <Switch on={c.enabled} onToggle={() => toggleEnabled(c.id, c.enabled)} />
                    <span style={{ fontSize: 'var(--text-control)', color: 'var(--text-2)', flex: 1 }}>启用此连接</span>
                    <div style={{ display: 'flex', gap: 8 }}>
                      <button className="btn btn-sm btn-danger" onClick={() => setConfirmDel(c.id)}><Icon name="trash" size={13} />删除</button>
                      <button className="btn btn-sm" onClick={() => testConn(c.id)} disabled={testing === c.id}>
                        <Icon name="zap" size={13} />{testing === c.id ? '测试中…' : '测试连接'}
                      </button>
                      <button className="btn btn-sm btn-primary" onClick={() => save(c.id)} disabled={Object.keys(d).length === 0 || saving === c.id}>
                        <Icon name="check" size={13} />{saving === c.id ? '保存中…' : '保存'}
                      </button>
                    </div>
                  </div>
                  {(saveStatus[c.id] || testResult[c.id]) && (
                    <div style={{ fontSize: 'var(--text-label)', fontFamily: 'var(--font-mono)', lineHeight: 'var(--leading-normal)',
                      color: (saveStatus[c.id] === '已保存' || (testResult[c.id] ?? '').startsWith('连接成功'))
                        ? 'var(--green-soft)' : 'var(--red)',
                    }}>
                      {saveStatus[c.id] ?? fmtTestResult(testResult[c.id])}
                    </div>
                  )}
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
  const [saveStatus, setSaveStatus] = useState<Record<string, string>>({});
  const [confirmDel, setConfirmDel] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const reload = () =>
    Promise.all([listAgents(), listLlmConfigs()]).then(([ags, llms]) => {
      setAgents(ags);
      setLlmNames(llms.map(l => ({ id: l.id, name: l.name })));
      setLoading(false);
    }).catch(() => setLoading(false));

  useEffect(() => { reload(); }, []);

  const setDraft = (id: string, field: string, val: unknown) =>
    setDrafts(d => ({ ...d, [id]: { ...d[id], [field]: val } }));

  const save = async (id: string) => {
    const d = drafts[id] ?? {};
    if (Object.keys(d).length === 0) return;
    try {
      const updated = await updateAgent(id, d);
      setAgents(as => as.map(a => a.id === id ? updated : a));
      setDrafts(d2 => { const n = { ...d2 }; delete n[id]; return n; });
      setSaveStatus(s => ({ ...s, [id]: '已保存' }));
      setTimeout(() => setSaveStatus(s => { const n = { ...s }; delete n[id]; return n; }), 2500);
    } catch (e) {
      setSaveStatus(s => ({ ...s, [id]: '保存失败: ' + String(e) }));
    }
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

  if (loading) return (
    <div className="set-inner">
      <div className="set-h">Agent 配置</div>
      <div style={{ color: 'var(--text-3)', marginTop: 20 }}>加载中…</div>
    </div>
  );

  const systemKindLabels: Record<string, string> = {
    planner: 'Planner 调度器',
    summarizer: 'Summarizer 总结器',
    doc_writer: 'Doc Writer 文档生成器',
    context_compressor: 'Context Compressor',
    material_ai: 'Material AI 物料助手',
    spec_writer: 'Spec Writer 规格生成器',
  };
  const systemKinds = (a: Agent) => (a.system_kind ?? '').split(',').map(v => v.trim()).filter(Boolean);

  return (
    <div className="set-inner rise">
      {confirmDel && <ConfirmModal msg="确认删除此 Agent？" onOk={() => doDelete(confirmDel)} onCancel={() => setConfirmDel(null)} />}
      <div className="set-h">Agent 配置</div>
      <div className="set-desc">配置 Agent 的名称、LLM、系统提示词及可用范围。角色绑定请前往「角色指派」页。</div>

      <div className="sec-kicker" style={{ marginBottom: 12 }}>全部 Agent · {agents.length}</div>
      {agents.map(a => {
        const d = drafts[a.id] ?? {};
        const v = (f: keyof Agent) => d[f as keyof typeof d] !== undefined
          ? String(d[f as keyof typeof d] ?? '')
          : String(a[f] ?? '');
        const roleTags = (a.forge_role ?? '').split(',').map(r =>
          r === 'analysis' ? '需求分析' : r === 'test' ? '测试' : null
        ).filter(Boolean) as string[];
        systemKinds(a).forEach(kind => {
          roleTags.unshift(systemKindLabels[kind] ?? kind);
        });
        const shownRoleTags = roleTags.slice(0, 2);
        const hiddenRoleCount = Math.max(0, roleTags.length - shownRoleTags.length);
        return (
          <div className="cfg-card" key={a.id} style={exp === a.id ? { borderColor: 'var(--ember-tint-strong)' } : {}}>
            <div className="cfg-top" onClick={() => setExp(exp === a.id ? null : a.id)} style={{ cursor: 'pointer' }}>
              <Avatar agent={a} size={40} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div className="cfg-name cfg-name-line">
                  <span className="cfg-name-text">{v('name')}</span>
                  <span className="cfg-role-tags">
                    {shownRoleTags.map(tag => <span key={tag} className="chip ember">{tag}</span>)}
                    {hiddenRoleCount > 0 && <span className="chip ember" title={roleTags.join(' · ')}>...</span>}
                  </span>
                </div>
                <div className="cfg-sub">{formatAgentSub(a, llmNames)}</div>
              </div>
              <Icon name={exp === a.id ? 'chevDown' : 'chevRight'} size={18} style={{ color: 'var(--text-3)' }} />
            </div>
            {exp === a.id && (
              <div className="rise" style={{ marginTop: 15 }}>
                <div className="cfg-fields">
                  <div className="field"><label>Agent 名称</label>
                    <input value={v('name')} onChange={e => setDraft(a.id, 'name', e.target.value)} />
                  </div>
                  <div className="field"><label>使用的 LLM</label>
                    <Select
                      value={d.llm_id !== undefined ? String(d.llm_id ?? '') : (a.llm_id ?? '')}
                      onChange={val => setDraft(a.id, 'llm_id', val || null)}
                      options={[{ value: '', label: '— 未指定 —' }, ...llmNames.map(l => ({ value: l.id, label: l.name }))]} />
                  </div>
                  <div className="field full"><label>职责标签</label>
                    <input value={v('role')} onChange={e => setDraft(a.id, 'role', e.target.value)} />
                  </div>
                  <div className="field full">
                    <label>可用范围</label>
                    <div style={{ display: 'flex', gap: 18, alignItems: 'center', flexWrap: 'wrap', padding: '8px 0' }}>
                      <label style={{ display: 'inline-flex', alignItems: 'center', gap: 8, color: 'var(--text-2)', fontSize: 'var(--text-control)' }}>
                        <Switch on={Boolean(d.enabled ?? a.enabled)} onToggle={() => setDraft(a.id, 'enabled', !(d.enabled ?? a.enabled))} />启用
                      </label>
                      <label style={{ display: 'inline-flex', alignItems: 'center', gap: 8, color: 'var(--text-2)', fontSize: 'var(--text-control)' }}>
                        <Switch on={Boolean(d.mentionable ?? a.mentionable)} onToggle={() => setDraft(a.id, 'mentionable', !(d.mentionable ?? a.mentionable))} />可拉入群聊
                      </label>
                      <label style={{ display: 'inline-flex', alignItems: 'center', gap: 8, color: 'var(--text-2)', fontSize: 'var(--text-control)' }}>
                        <Switch on={Boolean(d.visible_in_chat ?? a.visible_in_chat)} onToggle={() => setDraft(a.id, 'visible_in_chat', !(d.visible_in_chat ?? a.visible_in_chat))} />可私聊
                      </label>
                    </div>
                  </div>
                  <div className="field full"><label>系统提示词</label>
                    <textarea className="mono" rows={5} value={v('system_prompt')} onChange={e => setDraft(a.id, 'system_prompt', e.target.value)} />
                  </div>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 14, paddingTop: 14, borderTop: '1px solid var(--border)', justifyContent: 'flex-end' }}>
                  {saveStatus[a.id] && (
                    <span style={{ fontSize: 'var(--text-label)', fontFamily: 'var(--font-mono)', color: saveStatus[a.id] === '已保存' ? 'var(--green-soft)' : 'var(--red)', marginRight: 'auto' }}>
                      {saveStatus[a.id]}
                    </span>
                  )}
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

// ── Role Assignment ───────────────────────────────────────────────────────────
function RoleAssignment() {
  const [agents, setAgents] = useState<Agent[]>([]);
  const [saveStatus, setSaveStatus] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    listAgents().then(ags => { setAgents(ags); setLoading(false); }).catch(() => setLoading(false));
  }, []);

  const agentOpts = [{ value: '', label: '— 未指派 —' }, ...agents.map(a => ({ value: a.id, label: a.name }))];
  const analystId = agents.find(a => a.forge_role?.split(',').includes('analysis'))?.id ?? '';
  const testerId  = agents.find(a => a.forge_role?.split(',').includes('test'))?.id ?? '';

  const assignRole = async (val: string, role: 'analysis' | 'test') => {
    const fresh = await setAgentForgeRole(val, role);
    setAgents(fresh);
  };

  const systemKinds = (a: Agent) => (a.system_kind ?? '').split(',').map(v => v.trim()).filter(Boolean);
  const hasSystemKind = (a: Agent, kind: string) => systemKinds(a).includes(kind);
  const systemKindValue = (a: Agent, add?: string, remove?: string) => {
    const kinds = new Set(systemKinds(a));
    if (remove) kinds.delete(remove);
    if (add) kinds.add(add);
    return Array.from(kinds).join(',') || null;
  };

  const updateLocal = async (id: string, payload: Partial<Agent>, status = '已指派') => {
    try {
      const updated = await updateAgent(id, payload);
      setAgents(as => as.map(a => a.id === id ? updated : a));
      setSaveStatus(s => ({ ...s, [id]: status }));
      setTimeout(() => setSaveStatus(s => { const n = { ...s }; delete n[id]; return n; }), 2500);
    } catch (e) {
      setSaveStatus(s => ({ ...s, [id]: '保存失败: ' + String(e) }));
    }
  };

  const setSystemRoleAgent = async (kind: string, id: string) => {
    const current = agents.filter(a => hasSystemKind(a, kind));
    if (!id) {
      for (const holder of current) {
        await updateLocal(holder.id, { system_kind: systemKindValue(holder, undefined, kind) }, '已取消');
      }
      return;
    }
    for (const holder of current) {
      if (holder.id !== id) {
        await updateLocal(holder.id, { system_kind: systemKindValue(holder, undefined, kind) }, '已取消');
      }
    }
    const selected = agents.find(a => a.id === id);
    if (selected) {
      await updateLocal(id, { system_kind: systemKindValue(selected, kind) });
    }
  };

  const systemRoleRows = [
    { kind: 'planner',           title: 'Planner 调度器',              icon: 'layers', color: 'var(--blue)',   desc: '解析群聊自然语言请求，生成多 Agent 并发/串行编排计划' },
    { kind: 'summarizer',        title: 'Summarizer 总结器',            icon: 'quote',  color: 'var(--violet)', desc: '综合多个 Agent 的发言，形成结论、裁决和下一步建议' },
    { kind: 'doc_writer',        title: 'Doc Writer 文档生成器',        icon: 'file',   color: 'var(--amber)',  desc: '把讨论结果整理成 PRD、ADR、测试计划等文档产物' },
    { kind: 'context_compressor',title: 'Context Compressor 压缩器',   icon: 'layers', color: 'var(--green)',  desc: '压缩长对话和附件摘要，控制后续 Agent 的上下文质量与长度' },
    { kind: 'material_ai',       title: 'Material AI 物料助手',         icon: 'folder', color: 'var(--ember)',  desc: '驱动物料库 AI 搜索和 AI 整理，使用该 Agent 自身的 LLM 配置' },
    { kind: 'spec_writer',       title: 'Spec Writer 规格生成器',       icon: 'file',   color: 'var(--blue)',   desc: '驱动项目规格页 AI 一键生成，使用该 Agent 自身的 LLM 配置' },
  ];

  if (loading) return (
    <div className="set-inner">
      <div className="set-h">角色指派</div>
      <div style={{ color: 'var(--text-3)', marginTop: 20 }}>加载中…</div>
    </div>
  );

  const feedback = Object.values(saveStatus)[0];

  return (
    <div className="set-inner rise">
      <div className="set-h">角色指派</div>
      <div className="set-desc">将 Agent 绑定到编排系统和流水线的各个职责槽位。每次只有一个 Agent 持有某项角色；模型使用该 Agent 自身的 LLM 配置。</div>

      {feedback && (
        <div style={{ fontSize: 'var(--text-label)', fontFamily: 'var(--font-mono)', color: 'var(--green-soft)', marginBottom: 12 }}>{feedback}</div>
      )}

      <div className="panel" style={{ marginBottom: 22 }}>
        <div className="panel-head">
          <div className="panel-title"><Icon name="bot" size={16} style={{ color: 'var(--blue)' }} />系统角色指派</div>
          <div className="panel-sub">群聊编排引擎的内置职责</div>
        </div>
        <div style={{ padding: '4px 18px 14px' }}>
          {systemRoleRows.map(row => {
            const holder = agents.find(a => hasSystemKind(a, row.kind));
            return (
              <div className="assign-row" key={row.kind}>
                <div className="cfg-logo" style={{ background: row.color, width: 34, height: 34 }}><Icon name={row.icon} size={17} /></div>
                <div className="assign-info">
                  <div className="assign-title">{row.title}</div>
                  <div className="assign-desc">{row.desc}</div>
                </div>
                <Select style={{ width: 180 }} value={holder?.id ?? ''} options={agentOpts}
                  onChange={val => setSystemRoleAgent(row.kind, val)} />
              </div>
            );
          })}
        </div>
      </div>

      <div className="panel" style={{ marginBottom: 22 }}>
        <div className="panel-head">
          <div className="panel-title"><Icon name="sliders" size={16} style={{ color: 'var(--ember)' }} />流水线角色指派</div>
          <div className="panel-sub">需求流水线各阶段的 Agent 绑定</div>
        </div>
        <div style={{ padding: '4px 18px 14px' }}>
          <div className="assign-row">
            <div className="cfg-logo" style={{ background: 'var(--violet)', width: 34, height: 34 }}><Icon name="search" size={17} /></div>
            <div className="assign-info">
              <div className="assign-title">需求分析</div>
              <div className="assign-desc">在审核节点 1 前评估真实性、可行性、优先级</div>
            </div>
            <Select style={{ width: 180 }} value={analystId} options={agentOpts}
              onChange={val => assignRole(val, 'analysis')} />
          </div>
          <div className="assign-row">
            <div className="cfg-logo" style={{ background: 'var(--green)', width: 34, height: 34 }}><Icon name="flask" size={17} /></div>
            <div className="assign-info">
              <div className="assign-title">测试</div>
              <div className="assign-desc">合并后被动响应 + 每日主动巡检</div>
            </div>
            <Select style={{ width: 180 }} value={testerId} options={agentOpts}
              onChange={val => assignRole(val, 'test')} />
          </div>
        </div>
      </div>
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
            <Select value={form.queue_strategy} onChange={val => setForm(f => ({ ...f, queue_strategy: val }))}
              options={[{ value: 'priority', label: 'priority' }, { value: 'fifo', label: 'fifo' }, { value: 'oldest', label: 'oldest' }]} />
          </div>
          <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
            <button className="btn btn-primary" onClick={save}><Icon name="check" size={14} />保存配置</button>
            {result && <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{result}</span>}
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
            <Select value={name} onChange={setName}
              options={SPEC_FILES.map(f => ({ value: f, label: f }))} />
          </div>
          <div className="field full"><label>内容</label>
            <textarea className="mono" rows={18} value={content} onChange={e => setContent(e.target.value)} />
          </div>
          <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
            <button className="btn btn-primary" onClick={save}><Icon name="check" size={14} />保存文档</button>
            {status && <span style={{ fontSize: 'var(--text-label)', color: status === '已保存' ? 'var(--green-soft)' : 'var(--red)' }}>{status}</span>}
          </div>
        </div>
      </div>
    </div>
  );
}

function ThemeSettings({
  theme,
  onThemeChange,
}: {
  theme: ThemeSelection;
  onThemeChange: React.Dispatch<React.SetStateAction<ThemeSelection>>;
}) {
  const setMode = (mode: ThemeMode) => onThemeChange(t => ({ ...t, mode }));
  const selected = THEME_PALETTES.find(p => p.id === theme.palette) ?? THEME_PALETTES[0];

  return (
    <div className="set-inner set-inner-wide rise">
      <div className="set-h">主题设置</div>
      <div className="set-desc">当前明暗主题已归入 Forge Ember。选择任一主题族后，可在深色和浅色两种风格间切换。</div>

      <div className="theme-toolbar">
        <div>
          <div className="sec-kicker">当前主题</div>
          <div className="theme-current">{selected.name} · {theme.mode === 'dark' ? '深色' : '浅色'}</div>
        </div>
        <div className="theme-mode-toggle" aria-label="切换明暗风格">
          <button className={theme.mode === 'dark' ? 'active' : ''} onClick={() => setMode('dark')}>
            <Icon name="moon" size={14} />深色
          </button>
          <button className={theme.mode === 'light' ? 'active' : ''} onClick={() => setMode('light')}>
            <Icon name="sun" size={14} />浅色
          </button>
        </div>
      </div>

      <div className="theme-grid">
        {THEME_PALETTES.map(p => {
          const active = p.id === theme.palette;
          return (
            <div
              key={p.id}
              className={'theme-card' + (active ? ' active' : '')}
              style={{ '--theme-accent': p.accent } as React.CSSProperties}
            >
              <button className="theme-card-main" onClick={() => onThemeChange(t => ({ ...t, palette: p.id }))}>
                <div className="theme-preview">
                  <div className="theme-preview-rail" style={{ background: p.swatches[0] }}>
                    <i style={{ background: p.accent }} />
                    <i />
                    <i />
                  </div>
                  <div className="theme-preview-body">
                    <div className="theme-preview-line" />
                    <div className="theme-preview-panel">
                      <span style={{ background: p.accent }} />
                      <span />
                      <span />
                    </div>
                  </div>
                </div>
                <div className="theme-card-text">
                  <div>
                    <div className="theme-card-title">{p.name}</div>
                    <div className="theme-card-sub">{p.subtitle}</div>
                  </div>
                  <div className="theme-swatches">
                    {p.swatches.map(color => <i key={color} style={{ background: color }} />)}
                  </div>
                </div>
              </button>
              <div className="theme-card-actions">
                <button
                  className={active && theme.mode === 'dark' ? 'active' : ''}
                  onClick={() => onThemeChange({ palette: p.id, mode: 'dark' })}
                >
                  <Icon name="moon" size={13} />深色
                </button>
                <button
                  className={active && theme.mode === 'light' ? 'active' : ''}
                  onClick={() => onThemeChange({ palette: p.id, mode: 'light' })}
                >
                  <Icon name="sun" size={13} />浅色
                </button>
              </div>
            </div>
          );
        })}
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
        <div style={{ padding: '12px 18px', display: 'grid', gap: 8, fontSize: 'var(--text-control)', color: 'var(--text-2)' }}>
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
          {d.suggestions && <div style={{ padding: '0 2px 2px', fontSize: 'var(--text-control)', color: 'var(--text-3)' }}>{d.suggestions}</div>}
        </div>
      ))}
      {decisions.length === 0 && <div className="empty-compact" style={{ padding: '0' }}>暂无审计记录</div>}
    </div>
  );
}

function AboutSettings() {
  const [health, setHealth] = useState<SystemHealth | null>(null);
  const [healthError, setHealthError] = useState(false);
  const [healthLoading, setHealthLoading] = useState(true);
  const [previews, setPreviews] = useState<PreviewEnvironment[]>([]);
  const [tests, setTests] = useState<TestSession[]>([]);

  const loadHealth = () => {
    setHealthLoading(true);
    setHealthError(false);
    getSystemHealth()
      .then(h => { setHealth(h); setHealthError(false); })
      .catch(() => { setHealth(null); setHealthError(true); })
      .finally(() => setHealthLoading(false));
  };

  useEffect(() => {
    loadHealth();
    listPreviewEnvironments().then(setPreviews).catch(() => setPreviews([]));
    listTestSessions().then(setTests).catch(() => setTests([]));
  }, []);

  const dbVal   = healthLoading ? '…' : health?.db_ok ? 'OK' : healthError ? '错误' : '—';
  const dbColor = healthLoading ? 'var(--text-3)' : health?.db_ok ? 'var(--green)' : healthError ? 'var(--red)' : 'var(--text-3)';
  const authVal   = healthLoading ? '…' : health ? (health.claude_auth ? 'OK' : '未登录') : '—';
  const authColor = healthLoading ? 'var(--text-3)' : (health?.claude_auth) ? 'var(--green)' : health ? 'var(--red)' : 'var(--text-3)';

  return (
    <div className="set-inner rise">
      <div className="set-h">关于 AutoForge</div>
      <div className="set-desc" style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <span>运行健康、Claude 认证和后台运行态概览。</span>
        {healthError && <span style={{ fontSize: 'var(--text-label)', color: 'var(--red)' }}>状态获取失败</span>}
        <button className="btn" style={{ marginLeft: 'auto', fontSize: 'var(--text-label)', padding: '2px 10px' }} onClick={loadHealth} disabled={healthLoading}>
          {healthLoading ? '加载中…' : '刷新'}
        </button>
      </div>
      <div className="stat-grid" style={{ gridTemplateColumns: 'repeat(4, minmax(0, 1fr))', marginBottom: 16 }}>
        {[
          { label: '数据库',     val: dbVal,                         color: dbColor },
          { label: 'Claude Auth', val: authVal,                      color: authColor },
          { label: '版本',       val: health?.version ?? (healthLoading ? '…' : '—'), color: 'var(--blue)' },
          { label: '阶段',       val: health?.stage    ?? (healthLoading ? '…' : '—'), color: 'var(--ember)' },
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
          {previews.slice(0, 8).map(p => <div key={p.id} style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{p.status} · {p.preview_url || p.id}</div>)}
          {previews.length === 0 && <div className="empty-compact" style={{ padding: '0' }}>暂无预览环境</div>}
        </div>
      </div>
      <div className="panel">
        <div className="panel-head"><div className="panel-title"><Icon name="flask" size={16} style={{ color: 'var(--green)' }} />测试会话</div><span className="sec-kicker">{tests.length}</span></div>
        <div style={{ padding: '8px 18px 14px', display: 'grid', gap: 8 }}>
          {tests.slice(0, 8).map(t => <div key={t.id} style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)' }}>{t.status} · {t.summary || t.id}</div>)}
          {tests.length === 0 && <div className="empty-compact" style={{ padding: '0' }}>暂无测试会话</div>}
        </div>
      </div>
    </div>
  );
}

// ── WebhookSettings ───────────────────────────────────────────────────────────

function WebhookSettings() {
  const [cfg, setCfg]     = useState<IntakeConfig | null>(null);
  const [status, setStatus] = useState<WebhookStatus | null>(null);
  const [form, setForm]   = useState({ enabled: false, port: '27182', token: '' });
  const [saving, setSaving]   = useState(false);
  const [copied, setCopied]   = useState(false);
  const [saveOk, setSaveOk]   = useState<boolean | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    Promise.all([getIntakeConfig(), getWebhookStatus()])
      .then(([c, s]) => {
        setCfg(c);
        setStatus(s);
        setForm({ enabled: c.webhook_enabled, port: String(c.webhook_port), token: c.webhook_token });
      })
      .catch(() => {})
      .finally(() => setLoading(false));
  }, []);

  const save = async () => {
    setSaving(true); setSaveOk(null);
    try {
      const updated = await updateIntakeConfig({
        webhook_enabled: form.enabled,
        webhook_port: parseInt(form.port) || 27182,
        webhook_token: form.token,
      });
      setCfg(updated);
      setSaveOk(true);
      getWebhookStatus().then(setStatus).catch(() => {});
      setTimeout(() => setSaveOk(null), 2500);
    } catch { setSaveOk(false); }
    finally { setSaving(false); }
  };

  const genToken = () => {
    const bytes = new Uint8Array(24);
    crypto.getRandomValues(bytes);
    setForm(f => ({ ...f, token: btoa(String.fromCharCode(...bytes)).replace(/[+/=]/g, '') }));
  };

  const curlExample = `curl -X POST http://127.0.0.1:${form.port}/webhook/issues \\
  -H "Authorization: Bearer ${form.token || '<token>'}" \\
  -H "Content-Type: application/json" \\
  -d '{"project_id":"<uuid>","title":"需求标题","description":"详细描述"}'`;

  if (loading) return <div className="set-inner" style={{ color: 'var(--text-3)', fontSize: 'var(--text-control)' }}>加载中…</div>;

  return (
    <div className="set-inner" style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      {/* 状态卡 */}
      <div style={{ background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 14, padding: '18px 20px' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 18 }}>
          <div style={{ width: 34, height: 34, borderRadius: 9, background: 'rgba(232,119,46,.15)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Icon name="zap" size={16} style={{ color: 'var(--ember)' }} />
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ fontWeight: 600, fontSize: 'var(--text-body)' }}>HTTP Webhook 接口</div>
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 1 }}>外部系统通过 POST 请求推送需求到指定项目</div>
          </div>
          {status && (
            <span className={'chip ' + (status.running ? 'green' : '')} style={{ padding: '3px 10px', fontSize: 'var(--text-caption)' }}>
              <span style={{ width: 6, height: 6, borderRadius: '50%', background: status.running ? 'var(--green)' : 'var(--text-3)', display: 'inline-block', marginRight: 5 }} />
              {status.running ? `运行中 :${status.port}` : '已停止'}
            </span>
          )}
        </div>

        {/* 启用开关 */}
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '10px 14px', background: 'var(--bg-3)', borderRadius: 9, marginBottom: 14 }}>
          <div>
            <div style={{ fontWeight: 500, fontSize: 'var(--text-control)' }}>启用 Webhook</div>
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 1 }}>开启后外部系统可向本机端口推送需求</div>
          </div>
          <div role="switch" aria-checked={form.enabled} onClick={() => setForm(f => ({ ...f, enabled: !f.enabled }))}
            style={{ width: 38, height: 22, borderRadius: 11, cursor: 'pointer', flexShrink: 0, background: form.enabled ? 'var(--ember)' : 'var(--bg-3)', border: `1.5px solid ${form.enabled ? 'var(--ember-deep)' : 'var(--border-strong)'}`, position: 'relative', transition: 'background .2s, border-color .2s' }}>
            <div style={{ position: 'absolute', top: 2, left: form.enabled ? 18 : 2, width: 14, height: 14, borderRadius: '50%', background: '#fff', transition: 'left .2s cubic-bezier(.4,0,.2,1)', boxShadow: '0 1px 3px rgba(0,0,0,.35)' }} />
          </div>
        </div>

        <div className="cfg-fields" style={{ gridTemplateColumns: '120px 1fr', gap: 10, marginBottom: 14 }}>
          <div className="field" style={{ margin: 0 }}>
            <label>监听端口</label>
            <input value={form.port} onChange={e => setForm(f => ({ ...f, port: e.target.value }))} placeholder="27182" />
          </div>
          <div className="field" style={{ margin: 0 }}>
            <label>访问 Token</label>
            <div style={{ display: 'flex', gap: 6 }}>
              <input value={form.token} onChange={e => setForm(f => ({ ...f, token: e.target.value }))}
                placeholder="Bearer 令牌" style={{ flex: 1, fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)' }} />
              <button className="btn btn-sm" onClick={genToken} style={{ flexShrink: 0 }}>
                <Icon name="refresh" size={13} />生成
              </button>
            </div>
          </div>
        </div>

        <div style={{ background: 'rgba(139,122,216,.08)', border: '1px solid rgba(139,122,216,.22)', borderRadius: 10, padding: '10px 14px', fontSize: 'var(--text-label)', color: 'var(--text-2)', display: 'flex', gap: 8, marginBottom: 14 }}>
          <Icon name="bell" size={13} style={{ flexShrink: 0, marginTop: 1, color: 'var(--violet)' }} />
          <div>Webhook 仅监听 <code style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)' }}>127.0.0.1</code>（本机），不暴露公网。更改配置后点击「保存并应用」以重启服务。</div>
        </div>

        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <button className="btn btn-primary" onClick={save} disabled={saving}>
            <Icon name="check" size={14} />{saving ? '保存中…' : '保存并应用'}
          </button>
          {saveOk === true && <span style={{ fontSize: 'var(--text-label)', color: 'var(--green)' }}>✓ 已保存</span>}
          {saveOk === false && <span style={{ fontSize: 'var(--text-label)', color: 'var(--red)' }}>保存失败</span>}
        </div>
      </div>

      {/* curl 示例卡 */}
      <div style={{ background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 14, padding: '18px 20px' }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 10 }}>
          <div style={{ fontSize: 'var(--text-caption)', fontWeight: 700, letterSpacing: '.07em', textTransform: 'uppercase', color: 'var(--text-faint)' }}>curl 示例</div>
          <button className="icon-btn" style={{ width: 26, height: 26 }} title="复制"
            onClick={async () => { await navigator.clipboard.writeText(curlExample).catch(() => {}); setCopied(true); setTimeout(() => setCopied(false), 1200); }}>
            <Icon name={copied ? 'check' : 'copy'} size={13} />
          </button>
        </div>
        <pre style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', lineHeight: 'var(--leading-relaxed)', color: 'var(--text-2)', background: 'var(--bg-3)', borderRadius: 8, padding: '12px 14px', overflowX: 'auto', margin: 0, whiteSpace: 'pre-wrap', wordBreak: 'break-all', border: '1px solid var(--border)' }}>
          {curlExample}
        </pre>
      </div>
    </div>
  );
}

const SET_ITEMS = [
  { id: 'theme',       name: '主题设置',     ic: 'palette' },
  { id: 'llm',         name: 'LLM 配置',     ic: 'brain' },
  { id: 'agents',      name: 'Agent 配置',   ic: 'bot' },
  { id: 'roles',       name: '角色指派',     ic: 'layers' },
  { id: 'concurrency', name: '并发与流控',   ic: 'cpu' },
  { id: 'security',    name: '安全与权限',   ic: 'shield' },
  { id: 'webhook',     name: 'Webhook 集成', ic: 'zap' },
  { id: 'specs',       name: '规范文档',     ic: 'file' },
  { id: 'about',       name: '关于 AutoForge', ic: 'box' },
];

export default function SettingsPage({
  theme,
  onThemeChange,
}: {
  theme: ThemeSelection;
  onThemeChange: React.Dispatch<React.SetStateAction<ThemeSelection>>;
}) {
  const [sec, setSec] = useState('llm');
  const cur = SET_ITEMS.find(i => i.id === sec)!;
  return (
    <div className="content">
      <div className="audit-top" style={{ height: 56 }}>
        <div className="eyebrow" style={{ fontSize: 'var(--text-heading)' }}><span className="en">SETTINGS</span><span className="cn">· 设置</span></div>
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
          {sec === 'theme'       && <ThemeSettings theme={theme} onThemeChange={onThemeChange} />}
          {sec === 'llm'         && <LLMSettings />}
          {sec === 'agents'      && <AgentSettings />}
          {sec === 'roles'       && <RoleAssignment />}
          {sec === 'concurrency' && <ConcurrencySettings />}
          {sec === 'security'    && <SecuritySettings />}
          {sec === 'webhook'     && <WebhookSettings />}
          {sec === 'specs'       && <SpecsSettings />}
          {sec === 'about'       && <AboutSettings />}
          {!['theme','llm','agents','roles','concurrency','security','webhook','specs','about'].includes(sec) && (
            <div className="empty" style={{ height: '100%' }}>
              <Icon name={cur.ic} /><div>{cur.name}</div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
