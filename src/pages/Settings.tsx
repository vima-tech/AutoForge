import React, { useState, useEffect } from 'react';
import { createPortal } from 'react-dom';
import Icon from '../components/Icon';
import { Avatar } from '../components/Avatar';
import Select from '../components/Select';
import { THEME_PALETTES, type ThemeMode, type ThemeSelection } from '../theme';
import {
  listLlmConfigs, createLlmConfig, updateLlmConfig, deleteLlmConfig, testLlmConnection,
  listAgents, createAgent, updateAgent, deleteAgent,
  listRoleCatalog, setRoleSlot,
  getSystemHealth, updateConcurrencyConfig, readSpec, writeSpec,
  listPreviewEnvironments, listTestSessions, listAdminDecisions,
  getIntakeConfig, updateIntakeConfig, getWebhookStatus,
  listNotifyChannels, createNotifyChannel, deleteNotifyChannel, testNotifyChannel,
  listAutoPassPolicy, getAutoPassEnabled, setAutoPassEnabled,
  getKnowledgeSettings, setKnowledgeSettings,
  type LlmConfig, type Agent, type SystemHealth, type PreviewEnvironment,
  type TestSession, type AdminDecision, type IntakeConfig, type WebhookStatus,
  type NotifyChannel, type AutoPassPolicy, type RoleSlot,
} from '../services';

// ── helpers ──────────────────────────────────────────────────────────────────
function Switch({ on, onToggle }: { on: boolean; onToggle: () => void }) {
  return <button className={'switch' + (on ? ' on' : '')} onClick={onToggle}><i /></button>;
}

function ConfirmModal({ msg, onOk, onCancel }: { msg: string; onOk: () => void; onCancel: () => void }) {
  return createPortal(
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.5)', display: 'grid', placeItems: 'center', zIndex: 9999 }}>
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

// ── 对话角色（自定义业务 Agent，群聊/私聊用）────────────────────────────────────
// 起手模板，避免空白页。
const AGENT_TEMPLATES: { label: string; prompt: string }[] = [
  { label: '通用助手', prompt: '你是一个专业、严谨的助手。理解用户意图，给出结构化、可执行的回答；信息不足时主动澄清，不臆造。' },
  { label: '评审', prompt: '你是资深代码/方案评审者。逐项指出问题（正确性、边界、安全、可维护性），区分严重级别，并给出具体修改建议。' },
  { label: '文案', prompt: '你是产品文案专家。用简洁、准确、有说服力的中文表达，匹配目标受众与场景，避免空话套话。' },
  { label: '分析', prompt: '你是分析师。基于事实拆解问题，给出选项、取舍与明确建议，必要时量化，并标注假设与不确定性。' },
];

function CustomAgents({ onChanged }: { onChanged: () => void }) {
  const [agents, setAgents] = useState<Agent[]>([]);
  const [hideIds, setHideIds] = useState<Set<string>>(new Set());
  const [llmNames, setLlmNames] = useState<{ id: string; name: string }[]>([]);
  const [exp, setExp] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, Partial<Agent>>>({});
  const [saveStatus, setSaveStatus] = useState<Record<string, string>>({});
  const [confirmDel, setConfirmDel] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const reload = () =>
    Promise.all([listAgents(), listLlmConfigs(), listRoleCatalog()]).then(([ags, llms, cat]) => {
      setAgents(ags);
      setHideIds(new Set(cat.map(s => s.holder?.id).filter(Boolean) as string[]));
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
    onChanged();
  };

  const addNew = async () => {
    const a = await createAgent({ name: '新对话角色', system_prompt: AGENT_TEMPLATES[0].prompt, prompt_mode: 'custom', role_type: 'business' });
    setAgents(as => [...as, a]);
    setExp(a.id);
    onChanged();
  };

  // 仅展示未被系统/流水线角色占用的业务 Agent（其余在上方以角色卡管理）。
  const customAgents = agents.filter(a => !hideIds.has(a.id) && a.role_type !== 'system');

  if (loading) return <div style={{ color: 'var(--text-3)', marginTop: 12 }}>加载中…</div>;

  return (
    <div>
      {confirmDel && <ConfirmModal msg="确认删除此对话角色？" onOk={() => doDelete(confirmDel)} onCancel={() => setConfirmDel(null)} />}
      {customAgents.map(a => {
        const d = drafts[a.id] ?? {};
        const v = (f: keyof Agent) => d[f as keyof typeof d] !== undefined
          ? String(d[f as keyof typeof d] ?? '')
          : String(a[f] ?? '');
        return (
          <div className="cfg-card" key={a.id} style={{ padding: exp === a.id ? '13px 16px' : '8px 12px', marginBottom: 6, ...(exp === a.id ? { borderColor: 'var(--ember-tint-strong)' } : {}) }}>
            <div className="cfg-top" onClick={() => setExp(exp === a.id ? null : a.id)} style={{ cursor: 'pointer', gap: 10 }}>
              <Avatar agent={a} size={32} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div className="cfg-name cfg-name-line"><span className="cfg-name-text">{v('name')}</span></div>
                <div className="cfg-sub">{formatAgentSub(a, llmNames)}</div>
              </div>
              <Icon name={exp === a.id ? 'chevDown' : 'chevRight'} size={18} style={{ color: 'var(--text-3)' }} />
            </div>
            {exp === a.id && (
              <div className="rise" style={{ marginTop: 15 }}>
                <div className="cfg-fields">
                  <div className="field"><label>名称</label>
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
                  <div className="field full">
                    <label style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                      <span>系统提示词</span>
                      <span style={{ display: 'flex', gap: 4 }}>
                        {AGENT_TEMPLATES.map(t => (
                          <button key={t.label} className="btn btn-sm" style={{ padding: '2px 8px', fontSize: 'var(--text-micro)' }}
                            onClick={() => setDraft(a.id, 'system_prompt', t.prompt)}>{t.label}</button>
                        ))}
                      </span>
                    </label>
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
      <div className="cfg-card add" onClick={addNew} style={{ padding: 12, marginBottom: 0 }}><Icon name="plus" size={16} />新建对话角色</div>
    </div>
  );
}


// ── 系统角色卡（角色即 Agent，内置专业提示词）─────────────────────────────────
const PROMPT_MODES: { id: 'builtin' | 'append' | 'custom'; label: string }[] = [
  { id: 'builtin', label: '内置' },
  { id: 'append', label: '内置+补充' },
  { id: 'custom', label: '自定义' },
];

function RoleSlotCard({ slot, llms, onApply }: {
  slot: RoleSlot;
  llms: { id: string; name: string }[];
  onApply: (kind: string, payload: Parameters<typeof setRoleSlot>[1]) => Promise<void>;
}) {
  const h = slot.holder;
  const mode = (h?.prompt_mode ?? 'builtin') as 'builtin' | 'append' | 'custom';
  const [supplement, setSupplement] = useState(h?.system_prompt ?? '');
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState(false);
  const dirty = supplement !== (h?.system_prompt ?? '');

  const apply = async (payload: Parameters<typeof setRoleSlot>[1]) => {
    setBusy(true);
    try { await onApply(slot.kind, payload); } finally { setBusy(false); }
  };

  const status = !h ? { t: '未配置', c: '' }
    : !h.enabled ? { t: '已停用', c: '' }
    : !h.llm_id ? { t: '缺 LLM', c: 'amber' }
    : { t: '已启用', c: 'green' };
  const llmName = h?.llm_id ? (llms.find(l => l.id === h.llm_id)?.name ?? h.llm_id) : '未指定 LLM';
  const modeLabel = PROMPT_MODES.find(m => m.id === mode)?.label ?? '内置';

  return (
    <div className="cfg-card" style={{ padding: open ? '13px 16px' : '8px 12px', marginBottom: 0, ...(open ? { borderColor: 'var(--ember-tint-strong)' } : {}) }}>
      <div className="cfg-top" onClick={() => setOpen(o => !o)} style={{ cursor: 'pointer', gap: 10 }}>
        <div className="cfg-logo" style={{ background: slot.color, width: 28, height: 28 }}><Icon name={slot.icon} size={15} /></div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div className="cfg-name cfg-name-line">
            <span className="cfg-name-text">{slot.name}</span>
            <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginLeft: 6 }}>{slot.name_en}</span>
          </div>
          <div className="cfg-sub" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {open ? slot.desc : (h ? `${llmName} · 提示词 ${modeLabel}` : slot.desc)}
          </div>
        </div>
        {!open && h?.mentionable && <span className="chip" style={{ flexShrink: 0, fontSize: 'var(--text-micro)', padding: '1px 6px' }} title="可拉入群聊">群</span>}
        {!open && h?.visible_in_chat && <span className="chip" style={{ flexShrink: 0, fontSize: 'var(--text-micro)', padding: '1px 6px' }} title="可私聊">私</span>}
        {!open && h?.memory_enabled && <span className="chip ember" style={{ flexShrink: 0, fontSize: 'var(--text-micro)', padding: '1px 6px' }} title="已启用 Innate 记忆召回">记</span>}
        <span className={'chip ' + status.c} style={{ flexShrink: 0 }}>{status.t}</span>
        <Icon name={open ? 'chevDown' : 'chevRight'} size={18} style={{ color: 'var(--text-3)', flexShrink: 0 }} />
      </div>
      {open && (
      <div className="cfg-fields rise" style={{ marginTop: 14 }}>
        <div className="field"><label>使用的 LLM</label>
          <Select value={h?.llm_id ?? ''} options={[{ value: '', label: '— 未指定 —' }, ...llms.map(l => ({ value: l.id, label: l.name }))]}
            onChange={val => apply({ llm_id: val, enabled: true })} />
        </div>
        <div className="field"><label>提示词</label>
          <div className="seg">
            {PROMPT_MODES.map(m => (
              <button key={m.id} className={mode === m.id ? 'on' : ''} disabled={busy}
                onClick={() => apply({ prompt_mode: m.id })}>{m.label}</button>
            ))}
          </div>
        </div>
        {mode !== 'builtin' && (
          <div className="field full">
            <label>{mode === 'append' ? '补充指令（追加在内置提示词之后）' : '自定义提示词（替换内置）'}</label>
            <textarea className="mono" rows={4} value={supplement} onChange={e => setSupplement(e.target.value)}
              placeholder={mode === 'append' ? '例如：输出务必使用简体中文；遵守项目 DESIGN.md…' : '完全自定义该角色的系统提示词'} />
            <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 6 }}>
              <button className="btn btn-sm btn-primary" disabled={busy || !dirty} onClick={() => apply({ supplement })}>
                <Icon name="check" size={13} />保存提示词
              </button>
            </div>
          </div>
        )}
        <div className="field full">
          <label>可用范围</label>
          <div style={{ display: 'flex', gap: 18, alignItems: 'center', flexWrap: 'wrap', padding: '8px 0' }}>
            <label style={{ display: 'inline-flex', alignItems: 'center', gap: 8, color: 'var(--text-2)', fontSize: 'var(--text-control)' }}>
              <Switch on={Boolean(h?.enabled)} onToggle={() => apply({ enabled: !(h?.enabled) })} />启用
            </label>
            <label style={{ display: 'inline-flex', alignItems: 'center', gap: 8, color: 'var(--text-2)', fontSize: 'var(--text-control)' }}>
              <Switch on={Boolean(h?.mentionable)} onToggle={() => apply({ mentionable: !(h?.mentionable) })} />可拉入群聊
            </label>
            <label style={{ display: 'inline-flex', alignItems: 'center', gap: 8, color: 'var(--text-2)', fontSize: 'var(--text-control)' }}>
              <Switch on={Boolean(h?.visible_in_chat)} onToggle={() => apply({ visible_in_chat: !(h?.visible_in_chat) })} />可私聊
            </label>
            <label style={{ display: 'inline-flex', alignItems: 'center', gap: 8, color: 'var(--text-2)', fontSize: 'var(--text-control)' }} title="开启后该角色会召回本项目历史经验注入提示词，随使用越来越准（需安装 innate CLI）">
              <Switch on={h ? Boolean(h.memory_enabled) : true} onToggle={() => apply({ memory_enabled: !(h?.memory_enabled ?? true) })} />启用记忆
            </label>
          </div>
        </div>
      </div>
      )}
    </div>
  );
}

const ROLE_GROUPS: { id: RoleSlot['group']; title: string; icon: string; color: string; sub: string }[] = [
  { id: 'orchestration', title: '群聊编排角色', icon: 'bot',     color: 'var(--blue)',  sub: '会议室多 Agent 协作的内置职责' },
  { id: 'delivery',      title: '交付与项目角色', icon: 'package', color: 'var(--green)', sub: '交付流水线与项目工具的 AI 职责' },
  { id: 'pipeline',      title: '需求流水线角色', icon: 'sliders', color: 'var(--ember)', sub: '分析 / 测试阶段' },
];

function RoleCardsSection({ onChanged }: { onChanged: () => void }) {
  const [slots, setSlots] = useState<RoleSlot[]>([]);
  const [llms, setLlms] = useState<{ id: string; name: string }[]>([]);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState('');
  const [openGroups, setOpenGroups] = useState<Record<string, boolean>>({}); // 默认收起

  useEffect(() => {
    Promise.all([listRoleCatalog(), listLlmConfigs()]).then(([s, l]) => {
      setSlots(s); setLlms(l.map(x => ({ id: x.id, name: x.name }))); setLoading(false);
    }).catch(e => { setErr(String(e)); setLoading(false); });
  }, []);

  const apply = async (kind: string, payload: Parameters<typeof setRoleSlot>[1]) => {
    try { setSlots(await setRoleSlot(kind, payload)); onChanged(); }
    catch (e) { setErr(String(e)); }
  };

  if (loading) return <div style={{ color: 'var(--text-3)', marginTop: 12 }}>加载中…</div>;

  return (
    <div>
      {err && <div className="chip red" style={{ marginBottom: 12 }}><Icon name="alert" size={12} />{err}</div>}
      {ROLE_GROUPS.map(g => {
        const rows = slots.filter(s => s.group === g.id);
        if (rows.length === 0) return null;
        const open = !!openGroups[g.id];
        // 完整配置 = 有持有 Agent + 已启用 + 已绑定 LLM（缺一即不计入）
        const active = rows.filter(r => r.holder?.enabled && r.holder?.llm_id).length;
        const complete = active === rows.length;
        return (
          <div className="panel" style={{ marginBottom: 12 }} key={g.id}>
            <div className="panel-head" onClick={() => setOpenGroups(c => ({ ...c, [g.id]: !open }))} style={{ cursor: 'pointer' }}>
              <div className="panel-title">
                <Icon name={g.icon} size={16} style={{ color: g.color }} />{g.title}
                {!complete && <Icon name="alert" size={13} style={{ color: 'var(--amber)', marginLeft: 6 }} />}
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <span className={'chip ' + (complete ? 'green' : 'amber')} style={{ fontSize: 'var(--text-micro)', padding: '1px 7px' }}>{active}/{rows.length}</span>
                <Icon name={open ? 'chevDown' : 'chevRight'} size={16} style={{ color: 'var(--text-3)' }} />
              </div>
            </div>
            {open && (
              <div style={{ padding: '8px 12px', display: 'flex', flexDirection: 'column', gap: 6 }}>
                {rows.map(s => <RoleSlotCard key={s.kind} slot={s} llms={llms} onApply={apply} />)}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

function RolesPage() {
  const [bump, setBump] = useState(0);
  const [showCustom, setShowCustom] = useState(false);
  const onChanged = () => setBump(b => b + 1);
  return (
    <div className="set-inner rise">
      <div className="set-h">角色 Agent</div>
      <div className="set-desc">两层模型：角色即 Agent。系统角色自带专业内置提示词，选 LLM 即可启用，可"内置+补充"或自定义；对话角色用于群聊/私聊，可自由创建。</div>
      <RoleCardsSection onChanged={onChanged} />
      <div className="panel" style={{ marginBottom: 12 }}>
        <div className="panel-head" onClick={() => setShowCustom(v => !v)} style={{ cursor: 'pointer' }}>
          <div className="panel-title"><Icon name="bot" size={16} style={{ color: 'var(--violet)' }} />对话角色 · 自定义</div>
          <Icon name={showCustom ? 'chevDown' : 'chevRight'} size={16} style={{ color: 'var(--text-3)' }} />
        </div>
        {showCustom && (
          <div style={{ padding: '8px 12px' }}>
            <CustomAgents key={'ca' + bump} onChanged={onChanged} />
          </div>
        )}
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

function KnowledgeSettings() {
  const [form, setForm] = useState({ evolve_interval_hours: 12, capture_threshold: 8 });
  const [result, setResult] = useState('');

  useEffect(() => {
    getKnowledgeSettings().then(s => setForm(s)).catch(e => setResult(String(e)));
  }, []);

  const save = async () => {
    try {
      const s = await setKnowledgeSettings(form);
      setForm(s);
      setResult('已保存 · 后台自成长配置已生效');
    } catch (e) {
      setResult(String(e));
    }
  };

  return (
    <div className="set-inner rise">
      <div className="set-h">知识库自成长（Innate）</div>
      <div className="set-desc">
        AutoForge 在后台持续把交付经验蒸馏进各项目知识库。捕获达到阈值即自动进化，定时器作为低活跃项目的兜底；会议室里用 <code>/remember</code> <code>/recall</code> <code>/evolve</code> <code>/innate</code> 手动驱动。
      </div>
      <div className="cfg-card">
        <div className="cfg-fields">
          <div className="field"><label>定时进化间隔（小时）</label>
            <input type="number" min="0" max="720" value={form.evolve_interval_hours}
              onChange={e => setForm(f => ({ ...f, evolve_interval_hours: Number(e.target.value) }))} />
            <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>0 = 关闭定时器（仍按捕获阈值进化）</span>
          </div>
          <div className="field"><label>捕获阈值（次/项目）</label>
            <input type="number" min="0" max="1000" value={form.capture_threshold}
              onChange={e => setForm(f => ({ ...f, capture_threshold: Number(e.target.value) }))} />
            <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>累计这么多次捕获后自动进化；0 = 关闭事件触发</span>
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

const notifyInputStyle: React.CSSProperties = {
  background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 9,
  padding: '8px 10px', color: 'var(--text)', fontFamily: 'var(--font-sans)', fontSize: 'var(--text-control)',
};

function NotifySettings() {
  const [channels, setChannels] = useState<NotifyChannel[]>([]);
  const [form, setForm] = useState({ name: '', kind: 'slack', target: '' });
  const [busy, setBusy] = useState('');
  const [err, setErr] = useState('');

  const reload = () => { listNotifyChannels().then(setChannels).catch(() => setChannels([])); };
  useEffect(() => { reload(); }, []);

  const run = async (key: string, fn: () => Promise<unknown>) => {
    setErr(''); setBusy(key);
    try { await fn(); reload(); }
    catch (e) { setErr(String(e)); }
    finally { setBusy(''); }
  };

  return (
    <div className="set-inner rise">
      <div className="set-h">通知通道</div>
      <div className="set-desc">全局通知通道，用于推送流水线事件（审核、部署、安全告警等）。所有项目共享。</div>
      {err && <div className="chip red" style={{ alignSelf: 'flex-start', marginBottom: 12 }}><Icon name="alert" size={12} />{err}</div>}
      <div className="panel">
        <div className="panel-head">
          <div className="panel-title"><Icon name="bell" size={16} style={{ color: 'var(--ember)' }} />通知通道</div>
          <span className="sec-kicker">全局 · {channels.length} 个</span>
        </div>
        <div style={{ display: 'flex', gap: 8, padding: '12px 16px', borderTop: '1px solid var(--border)', alignItems: 'center', flexWrap: 'wrap' }}>
          <input value={form.name} onChange={e => setForm(f => ({ ...f, name: e.target.value }))} placeholder="名称" style={{ ...notifyInputStyle, width: 120 }} />
          <div style={{ minWidth: 130 }}>
            <Select value={form.kind} onChange={v => setForm(f => ({ ...f, kind: v }))}
              options={[{ value: 'slack', label: 'Slack' }, { value: 'wecom', label: '企业微信' }, { value: 'webhook', label: '通用 Webhook' }]} />
          </div>
          <input value={form.target} onChange={e => setForm(f => ({ ...f, target: e.target.value }))} placeholder="Webhook URL" style={{ ...notifyInputStyle, flex: 1, minWidth: 200 }} />
          <button className="btn btn-primary btn-sm" disabled={!form.name.trim() || !form.target.trim() || busy === 'add'}
            onClick={() => run('add', async () => { await createNotifyChannel(form); setForm({ name: '', kind: 'slack', target: '' }); })}>
            <Icon name="plus" size={14} />添加
          </button>
        </div>
        {channels.map(c => (
          <div key={c.id} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, padding: '10px 16px', borderTop: '1px solid var(--border)' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
              <span className="chip">{c.kind}</span>
              <span style={{ fontWeight: 600 }}>{c.name}</span>
              <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-faint)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', maxWidth: 260 }}>{c.target}</span>
            </div>
            <div style={{ display: 'flex', gap: 6 }}>
              <button className="btn btn-sm" disabled={busy === 'test' + c.id} onClick={() => run('test' + c.id, () => testNotifyChannel(c.kind, c.target))}><Icon name="send" size={13} />测试</button>
              <button className="btn btn-sm btn-danger" onClick={() => run('del' + c.id, () => deleteNotifyChannel(c.id))}><Icon name="trash" size={13} /></button>
            </div>
          </div>
        ))}
        {channels.length === 0 && <div className="empty-compact" style={{ padding: '14px 16px' }}>暂无通知通道</div>}
      </div>
    </div>
  );
}

function GatingSettings() {
  const [policies, setPolicies] = useState<AutoPassPolicy[]>([]);
  const [autoPassOn, setAutoPassOn] = useState(false);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState('');

  const reload = () => {
    listAutoPassPolicy().then(setPolicies).catch(() => setPolicies([]));
    getAutoPassEnabled().then(setAutoPassOn).catch(() => {});
  };
  useEffect(() => { reload(); }, []);

  const toggle = async () => {
    setErr(''); setBusy(true);
    try { const next = !autoPassOn; await setAutoPassEnabled(next); setAutoPassOn(next); }
    catch (e) { setErr(String(e)); }
    finally { setBusy(false); }
  };

  return (
    <div className="set-inner rise">
      <div className="set-h">门控降级</div>
      <div className="set-desc">全局自动放行策略。启用后低风险变更可在信任达标时跳过审核 2 自动合并。</div>
      {err && <div className="chip red" style={{ alignSelf: 'flex-start', marginBottom: 12 }}><Icon name="alert" size={12} />{err}</div>}
      <div className="panel">
        <div className="panel-head">
          <div className="panel-title"><Icon name="sliders" size={16} style={{ color: 'var(--ember)' }} />门控降级（自动放行）</div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span className={'chip ' + (autoPassOn ? 'green' : '')}>{autoPassOn ? '已启用' : '已关闭'}</span>
            <button className="btn btn-sm" disabled={busy} onClick={toggle}>
              <Icon name={autoPassOn ? 'pause' : 'play'} size={13} />{autoPassOn ? '关闭' : '启用'}
            </button>
          </div>
        </div>
        <div style={{ padding: '10px 16px', fontSize: 'var(--text-control)', color: 'var(--text-3)', borderTop: '1px solid var(--border)' }}>
          启用后，低风险(T0/T1)且变更类信任达标（连续 20 次批准、0 退改）的改动将自动跳过审核 2 直接合并；T3 硬地板（迁移/auth/支付/依赖）永远人工。任一退改清零重挣。
        </div>
        {policies.length === 0
          ? <div className="empty-compact" style={{ padding: '14px 16px' }}>暂无变更类信任记录</div>
          : policies.map(p => (
            <div key={p.change_class} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, padding: '9px 16px', borderTop: '1px solid var(--border)' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <span className={'chip ' + (p.trust_state === 'auto' ? 'green' : p.trust_state === 'eligible' ? 'amber' : '')}>{p.trust_state}</span>
                <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-control)' }}>{p.change_class}</span>
              </div>
              <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>批准连胜 {p.approve_count} · 退改 {p.reject_count}</span>
            </div>
          ))}
      </div>
    </div>
  );
}

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
  { id: 'roles',       name: '角色 Agent',         ic: 'bot' },
  { id: 'concurrency', name: '并发与流控',   ic: 'cpu' },
  { id: 'knowledge',   name: '知识库自成长', ic: 'brain' },
  { id: 'security',    name: '安全与权限',   ic: 'shield' },
  { id: 'webhook',     name: 'Webhook 集成', ic: 'zap' },
  { id: 'notify',      name: '通知通道',     ic: 'bell' },
  { id: 'gating',      name: '门控降级',     ic: 'sliders' },
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
          {sec === 'roles'       && <RolesPage />}
          {sec === 'concurrency' && <ConcurrencySettings />}
          {sec === 'knowledge'   && <KnowledgeSettings />}
          {sec === 'security'    && <SecuritySettings />}
          {sec === 'webhook'     && <WebhookSettings />}
          {sec === 'notify'      && <NotifySettings />}
          {sec === 'gating'      && <GatingSettings />}
          {sec === 'specs'       && <SpecsSettings />}
          {sec === 'about'       && <AboutSettings />}
          {!['theme','llm','roles','concurrency','knowledge','security','webhook','notify','gating','specs','about'].includes(sec) && (
            <div className="empty" style={{ height: '100%' }}>
              <Icon name={cur.ic} /><div>{cur.name}</div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
