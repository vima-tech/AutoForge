import React, { useState, useEffect, useRef, Fragment } from 'react';
import { listen } from '@tauri-apps/api/event';
import { createPortal } from 'react-dom';
import Icon from '../components/Icon';
import { Avatar } from '../components/Avatar';
import { refreshAgents } from '../agents-store';
import Select from '../components/Select';
import { fmtFull } from '../utils/datetime';
import {
  THEME_PALETTES, RAIL_STORAGE_KEY, applyRailMode, parseRailMode,
  RES_MONITOR_KEY, RES_MONITOR_CHANGED_EVENT, parseResMonitor,
  QUICK_CAPTURE_SHORTCUT_KEY, VOICE_INPUT_SHORTCUT_KEY, SHORTCUT_CHANGED_EVENT,
  DEFAULT_QUICK_CAPTURE_SHORTCUT, DEFAULT_VOICE_INPUT_SHORTCUT,
  parseQuickCaptureShortcut, parseVoiceInputShortcut, formatCombo, isModifierCode, comboHasModifier,
  type ThemeMode, type ThemeSelection, type RailMode, type QuickCaptureShortcut, type ShortcutCombo,
} from '../theme';
import {
  listLlmConfigs, createLlmConfig, updateLlmConfig, deleteLlmConfig, testLlmConnection,
  listAgents, createAgent, updateAgent, deleteAgent,
  listRoleCatalog, setRoleSlot,
  getSystemHealth, checkClaudeAuth, updateConcurrencyConfig, getConcurrencyConfig,
  listPreviewEnvironments, listTestSessions, listAdminDecisions, listJobFailures,
  getIntakeConfig, updateIntakeConfig, getWebhookStatus,
  listNotifyChannels, createNotifyChannel, updateNotifyChannel, deleteNotifyChannel, testNotifyChannel,
  clawbotStartLogin, clawbotPollLogin,
  listAutoPassPolicy, getAutoPassEnabled, setAutoPassEnabled,
  getAutoConflictResolveEnabled, setAutoConflictResolveEnabled,
  getCustomMergeMessageEnabled, setCustomMergeMessageEnabled,
  getParallelPremergeEnabled, setParallelPremergeEnabled,
  getKnowledgeSettings, setKnowledgeSettings,
  getKnowledgeEmbedding, setKnowledgeEmbedding,
  listProjects, selfUpdateStatus, selfUpdatePull, selfUpdatePending,
  getWebSearchSettings, setWebSearchSettings,
  getOpenDesignSettings, setOpenDesignSettings, getOpenDesignLog, type OpenDesignSettings,
  getAsrSettings, setAsrSettings, type AsrSettings as AsrSettingsT,
  getAutosupplySettings, setAutosupplySettings, runAutosupplyNow, autosupplyIsRunning, type AutosupplySettings as AutosupplyT,
  getAutonomyLevel, setAutonomyLevel, type AutonomyLevel,
  listBuiltinTools, type BuiltinToolInfo,
  getSecretBackendStatus, type SecretBackend,
  exportConfig, importConfig, revealBackup, type BackupSummary, type ExportResult,
  listMcpServers, createMcpServer, updateMcpServer, deleteMcpServer, testMcpConnection, discoverCodeIntelMap,
  type McpServer, type McpServerInput, type McpTransport,
  type LlmConfig, type Agent, type SystemHealth, type PreviewEnvironment,
  type TestSession, type AdminDecision, type IntakeConfig, type WebhookStatus,
  type NotifyChannel, type AutoPassPolicy, type RoleSlot, type EmbeddingSettings,
  type WebSearchSettings,
  type Project, type SelfUpdateStatus, type SelfUpdateResult,
  type JobFailure,
  listCodeAgents, upsertCodeAgent, deleteCodeAgent, setDefaultCodeAgent,
  setProjectCodeAgent, checkCodeAgentAuth, type CodeAgent as CodeAgentT, type CodeAgentProbe,
  listCodeAgentSkills, upsertCodeAgentSkill, deleteCodeAgentSkill,
  type CodeAgentSkill as CodeAgentSkillT,
} from '../services';

// ── helpers ──────────────────────────────────────────────────────────────────
function Switch({ on, onToggle }: { on: boolean; onToggle: () => void }) {
  // type=button 避免默认 submit 行为；stopPropagation/preventDefault 杜绝 WebKitGTK 下
  // <button> 内嵌 <label> 时父级 label 二次激活导致的“点了没反应/瞬间回弹”。
  return (
    <button
      type="button"
      className={'switch' + (on ? ' on' : '')}
      onClick={e => { e.preventDefault(); e.stopPropagation(); onToggle(); }}
    >
      <i />
    </button>
  );
}

function ConfirmModal({ msg, onOk, onCancel }: { msg: string; onOk: () => void; onCancel: () => void }) {
  return createPortal(
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', display: 'grid', placeItems: 'center', zIndex: 9999 }}>
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

const llmColor = (apiSpec: string) => {
  const p = (apiSpec || '').toLowerCase();
  if (p.includes('anthropic')) return '#8b7ad8';
  if (p.includes('openai')) return '#4f8ed1';
  if (p.includes('ollama')) return '#4f9d6b';
  return '#e8772e';
};

const API_SPEC_LABEL: Record<string, string> = {
  openai: 'OpenAI 兼容', anthropic: 'Anthropic',
};

type LlmRef = { id: string; name: string; enabled: boolean };

// 角色/Agent 绑定的 LLM 健康状态：未绑定 / 正常 / 已停用 / 配置缺失。
function llmBindingState(llmId: string | null | undefined, llms: LlmRef[]): 'none' | 'ok' | 'disabled' | 'missing' {
  if (!llmId) return 'none';
  const m = llms.find(l => l.id === llmId);
  if (!m) return 'missing';
  return m.enabled ? 'ok' : 'disabled';
}

// 下拉项标签：已停用的 LLM 追加标记，避免误选到不可用配置。
const llmOptionLabel = (l: LlmRef) => l.enabled ? l.name : `${l.name}（已停用）`;

function formatAgentSub(agent: Agent, llms: LlmRef[]) {
  if (!agent.llm_id) return 'LLM: 未指定 LLM';
  const m = llms.find(l => l.id === agent.llm_id);
  if (!m) return `LLM: ${agent.llm_id}（配置缺失）`;
  return `LLM: ${m.name}${m.enabled ? '' : '（已停用）'}`;
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
  const [secretBackend, setSecretBackend] = useState<SecretBackend | null>(null);
  // 切页会卸载本组件，但测试连接等异步 IPC 仍在飞行；用此 ref 拦截卸载后的 setState，
  // 避免无效更新与告警。准确的模型信息已落库，重新进入页面时 useEffect 会重新拉取。
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);

  useEffect(() => {
    listLlmConfigs().then(cs => { if (mounted.current) { setConfigs(cs); setLoading(false); } }).catch(() => { if (mounted.current) setLoading(false); });
    getSecretBackendStatus().then(s => { if (mounted.current) setSecretBackend(s); }).catch(() => {});
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
    try {
      // 测试连接同时刷新上下文窗口与多模态能力，回灌到配置展示。
      const { message, config } = await testLlmConnection(id, drafts[id]);
      if (!mounted.current) return; // 已切页：结果已落库，下次进页面会重新拉取
      setTestResult(r => ({ ...r, [id]: message }));
      setConfigs(cs => cs.map(c => c.id === id
        ? { ...c, ctx_window: config.ctx_window, supports_vision: config.supports_vision }
        : c));
    } catch (e) {
      if (mounted.current) setTestResult(r => ({ ...r, [id]: String(e) }));
    } finally {
      if (mounted.current) setTesting(null);
    }
  };

  const doDelete = async (id: string) => {
    await deleteLlmConfig(id);
    setConfigs(cs => cs.filter(c => c.id !== id));
    setConfirmDel(null);
  };

  const addNew = async () => {
    const c = await createLlmConfig({
      name: '新 LLM 配置', api_spec: 'anthropic',
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
      {secretBackend && (
        <div className="chip" style={{ marginBottom: 12 }}
          title={secretBackend === 'keychain'
            ? 'API Key 经 AES-256-GCM 加密落库，主密钥保存在系统钥匙环'
            : '未检测到系统钥匙环：主密钥退化保存为 0600 本地文件，安全性弱于钥匙环'}>
          <Icon name={secretBackend === 'keychain' ? 'shield' : 'alert'} size={11} style={{ verticalAlign: -1, marginRight: 4 }} />
          {secretBackend === 'keychain' ? '密钥加密：系统钥匙环' : '密钥加密：文件兜底（无钥匙环）'}
        </div>
      )}
      {configs.map(c => {
        const d = drafts[c.id] ?? {};
        const v = (f: keyof LlmConfig) => (d as Record<string, unknown>)[f] !== undefined ? (d as Record<string, unknown>)[f] as string : c[f] as string;
        return (
          <div className="cfg-card" key={c.id} style={exp === c.id ? { borderColor: 'var(--ember-tint-strong)' } : {}}>
            <div className="cfg-top" onClick={() => setExp(exp === c.id ? null : c.id)} style={{ cursor: 'pointer' }}>
              <div className="cfg-logo" style={{ background: llmColor(v('api_spec')) }}><Icon name="brain" size={20} /></div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div className="cfg-name">{v('name')}</div>
                <div className="cfg-sub">{API_SPEC_LABEL[v('api_spec')] ?? v('api_spec')} · {v('model')}</div>
              </div>
              <span className={'chip ' + (c.enabled ? 'green' : '')}>{c.enabled ? '● 已启用' : '未启用'}</span>
              <Icon name={exp === c.id ? 'chevDown' : 'chevRight'} size={18} style={{ color: 'var(--text-3)', marginLeft: 4 }} />
            </div>
            {exp === c.id && (
              <div className="cfg-fields rise">
                <div className="field full"><label>名称</label><input value={v('name')} onChange={e => setDraft(c.id, 'name', e.target.value)} /></div>
                <div className="field"><label>接口规范 · 工具调用格式</label>
                  <Select value={v('api_spec') || 'openai'} onChange={val => setDraft(c.id, 'api_spec', val)}
                    options={[
                      { value: 'openai', label: 'OpenAI 兼容' },
                      { value: 'anthropic', label: 'Anthropic' },
                    ]} />
                </div>
                <div className="field"><label>Model</label><input className="mono" value={v('model')} onChange={e => setDraft(c.id, 'model', e.target.value)} /></div>
                <div className="field full"><label>API Endpoint</label><input className="mono" value={v('endpoint')} onChange={e => setDraft(c.id, 'endpoint', e.target.value)} /></div>
                <div className="field full"><label><Icon name="key" size={11} style={{ verticalAlign: -1, marginRight: 4 }} />API Key</label>
                  <input className="mono" value={v('api_key')} onChange={e => setDraft(c.id, 'api_key', e.target.value)} type="password" />
                </div>
                <div className="field"><label>上下文窗口 · 自动</label>
                  <div className="mono" title="由后端按模型查表 + 接口探测自动得出；创建与修改模型/接口后自动刷新"
                    style={{ padding: '9px 11px', color: 'var(--text-2)', fontSize: 'var(--text-control)',
                      background: 'var(--bg-2)', border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)' }}>
                    {v('ctx_window') || '未知'}
                  </div>
                </div>
                <div className="field"><label>Temperature</label>
                  <input type="number" step="0.1" min="0" max="2"
                    value={d.temperature !== undefined ? String(d.temperature) : String(c.temperature)}
                    onChange={e => setDraft(c.id, 'temperature', parseFloat(e.target.value))} />
                </div>
                <div className="field full">
                  <label><Icon name="image" size={11} style={{ verticalAlign: -1, marginRight: 4 }} />多模态 · 图片识别 · 自动</label>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                    <span className={'chip ' + (c.supports_vision ? 'green' : '')}>
                      {c.supports_vision ? '● 支持' : '不支持'}
                    </span>
                    <span style={{ fontSize: 'var(--text-control)', color: 'var(--text-3)', flex: 1 }}>
                      由后端按模型名自动识别；创建与修改模型后自动更新。支持时，绑定此 LLM 的 Agent 可识别会议室中的图片附件。
                    </span>
                  </div>
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

// capabilities_json 工具白名单：规范形状为 {"tools":[...]}（后端 allowed_tools_from_capabilities
// 也只读 .tools）。但历史数据里它存的是扁平语义标签数组（如 ["planning","routing"]），
// 这类数组从未被工具系统消费。下方读取时一律归一，写入时统一输出 {"tools":[...]}——
// 这样首次切换即把旧数组迁移成对象形状，开关才真正生效。
// 注意：旧实现 `obj.tools=[...]` 若 obj 是数组，JSON.stringify(数组) 会丢弃该属性 → 改动丢失（点了没反应）。
function capTools(capJson: string | undefined): string[] {
  try {
    const v = JSON.parse(capJson || '{}') as unknown;
    if (Array.isArray(v)) return []; // 旧版扁平标签数组：不含任何工具
    const arr = (v as { tools?: unknown })?.tools;
    return Array.isArray(arr) ? arr.filter((x): x is string => typeof x === 'string') : [];
  } catch { return []; }
}
function agentHasTool(capJson: string | undefined, tool: string): boolean {
  return capTools(capJson).includes(tool);
}
function toggleAgentTool(capJson: string | undefined, tool: string, on: boolean): string {
  const set = new Set<string>(capTools(capJson));
  if (on) set.add(tool); else set.delete(tool);
  return JSON.stringify({ tools: [...set] });
}

// 内置工具目录：进程内缓存一次，所有能力开关从后端目录动态渲染——后端新增工具自动出现。
let _builtinToolsCache: BuiltinToolInfo[] | null = null;
function useBuiltinTools(): BuiltinToolInfo[] {
  const [tools, setTools] = useState<BuiltinToolInfo[]>(_builtinToolsCache ?? []);
  useEffect(() => {
    if (_builtinToolsCache) return;
    listBuiltinTools().then(t => { _builtinToolsCache = t; setTools(t); }).catch(() => {});
  }, []);
  return tools;
}

// ── 语音录入（ASR）设置 ───────────────────────────────────────────────────────
function AsrSettings() {
  const [cfg, setCfg] = useState<AsrSettingsT>({ provider: 'aliyun', endpoint: '', model: '', language: 'zh', api_key_set: false });
  const [apiKey, setApiKey] = useState('');
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState('');

  useEffect(() => { getAsrSettings().then(c => setCfg(c)).catch(e => setStatus(String(e))); }, []);

  // 全部语音识别统一走阿里百炼 DashScope，只需 API Key。
  const enabled = cfg.api_key_set || apiKey.trim().length > 0;

  const save = async () => {
    setBusy(true); setStatus('');
    try {
      // 收敛到百炼：provider 固定 aliyun、endpoint 不再使用。
      const r = await setAsrSettings('aliyun', '', cfg.model, cfg.language, apiKey.trim() || undefined);
      setCfg(r); setApiKey(''); setStatus('已保存');
    } catch (e) { setStatus(String(e)); }
    finally { setBusy(false); }
  };

  return (
    <div className="set-inner rise">
      <div className="set-h">语音录入</div>
      <div className="set-desc">配置语音识别（ASR）。所有语音识别——<strong>实时麦克风口述</strong>（速录/会议室边说边出字）与<strong>会议录音上传</strong>（整场录音转写+拆需求）——统一走<strong>阿里百炼 ASR</strong>（DashScope 实时流式，模型默认 paraformer-realtime-v2），只需填百炼 API Key，无需 Endpoint。结果视为不可信外部输入，提交时自动过安全过滤。</div>

      <div className="cfg-card" style={{ borderColor: enabled ? 'var(--ember-tint-strong)' : undefined }}>
        <div className="cfg-top" style={{ gap: 10 }}>
          <div className="cfg-logo" style={{ background: 'var(--ember)', width: 28, height: 28 }}><Icon name="mic" size={15} /></div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div className="cfg-name cfg-name-line"><span className="cfg-name-text">语音转写 · ASR</span>
              <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginLeft: 6 }}>INPUT</span>
            </div>
            <div className="cfg-sub">口述需求 → 转写文本 → 可改错别字后提交</div>
          </div>
          <span className={'chip ' + (enabled ? 'green' : 'amber')} style={{ flexShrink: 0 }}>{enabled ? '已启用' : '未配置'}</span>
        </div>

        <div className="cfg-fields rise" style={{ marginTop: 14 }}>
          <div className="field"><label>模型（可空，默认 paraformer-realtime-v2）</label>
            <input type="text" className="mono" value={cfg.model} placeholder="paraformer-realtime-v2"
              onChange={e => setCfg(c => ({ ...c, model: e.target.value }))} />
          </div>
          <div className="field"><label>语言（可空，自动检测）</label>
            <input type="text" className="mono" value={cfg.language} placeholder="zh"
              onChange={e => setCfg(c => ({ ...c, language: e.target.value }))} />
          </div>
          <div className="field full"><label><Icon name="key" size={11} style={{ verticalAlign: -1, marginRight: 4 }} />API Key</label>
            <input type="password" className="mono" value={apiKey}
              placeholder={cfg.api_key_set ? '已设置（留空则不修改）' : 'sk-...'}
              onChange={e => setApiKey(e.target.value)} />
          </div>
          <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
            <button className="btn btn-primary" disabled={busy} onClick={save}><Icon name="check" size={14} />保存语音配置</button>
            {status && <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{status}</span>}
          </div>
        </div>
      </div>
    </div>
  );
}

// ── 工厂自喂料（autosupply）设置 ──────────────────────────────────────────────
function AutosupplySettings() {
  const [cfg, setCfg] = useState<AutosupplyT>({ enabled: false, interval_min: 1440, scan_enabled: true, proposer_enabled: false, max_per_run: 20, proposer_max_per_run: 8, min_severity: 'low', analyze_enabled: true, triage_enabled: true });
  const [level, setLevel] = useState<AutonomyLevel>('strict');
  const [busy, setBusy] = useState(false);
  const [running, setRunning] = useState(false);
  const [status, setStatus] = useState('');

  useEffect(() => {
    getAutosupplySettings().then(setCfg).catch(e => setStatus(String(e)));
    getAutonomyLevel().then(setLevel).catch(() => {});
    // 状态真源在后端：切页重挂载时查询当前是否有一轮在跑，恢复「运行中」回显，
    // 并订阅 AutosupplyStatus 事件实时跟随开始/结束（手动或周期调度均覆盖）。
    autosupplyIsRunning().then(setRunning).catch(() => {});
    let unlisten: (() => void) | undefined;
    listen<{ type?: string; running?: boolean }>('autoforge://event', e => {
      if (e.payload?.type === 'autosupply_status') setRunning(!!e.payload.running);
    }).then(fn => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, []);

  const changeLevel = async (l: AutonomyLevel) => {
    setLevel(l);
    try { await setAutonomyLevel(l); setCfg(await getAutosupplySettings()); setStatus('信任档位已应用'); }
    catch (e) { setStatus(String(e)); }
  };

  const save = async (next: AutosupplyT) => {
    setCfg(next); setBusy(true); setStatus('');
    try { setCfg(await setAutosupplySettings(next)); setStatus('已保存'); }
    catch (e) { setStatus(String(e)); }
    finally { setBusy(false); }
  };
  const runNow = async () => {
    setRunning(true); setStatus('');
    try { const r = await runAutosupplyNow(); setStatus(`本轮入待整理池 ${r.new_issues} 条`); }
    catch (e) { setStatus(String(e)); }
    finally { setRunning(false); }
  };

  return (
    <div className="set-inner rise">
      <div className="set-h">自动供料</div>
      <div className="set-desc">让工厂自己找活干：周期性扫描代码 + proposer 主动提议改进，产物<strong>全部进「待整理池」，绝不自动进流水线</strong>，等你在功能审计里整理确认。两道人工审核闸在任何档位都保留。</div>

      <div className="cfg-card" style={{ marginBottom: 14 }}>
        <div className="field full" style={{ margin: 0 }}>
          <label>信任档位（autonomy）</label>
          <Select value={level} onChange={v => changeLevel(v as AutonomyLevel)}
            options={[
              { value: 'strict', label: 'Strict · 最紧（proposer 关 · 每轮 10 · 推荐）' },
              { value: 'standard', label: 'Standard · 标准（proposer 关 · 每轮 20）' },
              { value: 'loose', label: 'Loose · 放手（proposer 开 · 每轮 40）' },
            ]} />
          <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginTop: 6 }}>档位应用预设到下方自喂料参数；不改变审核闸/并发/合并——裁决权始终在你手里。信任长出来再往上拧。</span>
        </div>
      </div>

      <div className="cfg-card" style={{ borderColor: cfg.enabled ? 'var(--ember-tint-strong)' : undefined }}>
        <div className="cfg-fields rise" style={{ marginTop: 0 }}>
          <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 12, margin: 0 }}>
            <Switch on={cfg.enabled} onToggle={() => save({ ...cfg, enabled: !cfg.enabled })} />
            <span style={{ fontWeight: 600 }}>启用周期自喂料</span>
          </div>
          <div className="field"><label>间隔（分钟）</label>
            <input type="number" min="5" value={cfg.interval_min}
              onChange={e => setCfg(c => ({ ...c, interval_min: Math.max(5, Number(e.target.value) || 1440) }))}
              onBlur={() => save(cfg)} />
          </div>
          <div className="field"><label>扫描每轮入池上限</label>
            <input type="number" min="1" max="200" value={cfg.max_per_run}
              onChange={e => setCfg(c => ({ ...c, max_per_run: Math.max(1, Math.min(200, Number(e.target.value) || 20)) }))}
              onBlur={() => save(cfg)} />
          </div>
          <div className="field"><label>严重度门槛</label>
            <Select value={cfg.min_severity} onChange={v => save({ ...cfg, min_severity: v })}
              options={[
                { value: 'low', label: '全部（low+，不过滤）' },
                { value: 'medium', label: 'medium 及以上（滤掉皮毛）' },
                { value: 'high', label: 'high 及以上（只要要紧的）' },
                { value: 'critical', label: '仅 critical（最严苛）' },
              ]} />
            <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginTop: 6 }}>低于此级别的产物入池前丢弃，进一步压制 linter 噪音。</span>
          </div>
          <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 12, margin: 0 }}>
            <Switch on={cfg.scan_enabled} onToggle={() => save({ ...cfg, scan_enabled: !cfg.scan_enabled })} />
            <span>代码扫描（TODO / cargo · npm · pip · go 依赖审计）</span>
          </div>
          <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 12, margin: 0 }}>
            <Switch on={cfg.analyze_enabled} onToggle={() => save({ ...cfg, analyze_enabled: !cfg.analyze_enabled })} />
            <span>静态代码分析<span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginLeft: 6 }}>（按栈跑 clippy / ruff / go vet / eslint，发现真实代码问题；编译型较耗时）</span></span>
          </div>
          <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 12, margin: 0 }}>
            <Switch on={cfg.triage_enabled} onToggle={() => save({ ...cfg, triage_enabled: !cfg.triage_enabled })} />
            <span>前置整理（自动滤噪）<span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginLeft: 6 }}>（入池即跑 triage 滤掉噪音、归一化，幸存条目仍候人工闸口）</span></span>
          </div>
          <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 12, margin: 0 }}>
            <Switch on={cfg.proposer_enabled} onToggle={() => save({ ...cfg, proposer_enabled: !cfg.proposer_enabled })} />
            <span>proposer 深度审计提议<span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginLeft: 6 }}>（资深代码审计师，找 linter 发现不了的深层问题；独立预算，不被扫描挤占；建议绑定强模型）</span></span>
          </div>
          {cfg.proposer_enabled && (
            <div className="field"><label>proposer 每轮上限（独立预算）</label>
              <input type="number" min="1" max="50" value={cfg.proposer_max_per_run}
                onChange={e => setCfg(c => ({ ...c, proposer_max_per_run: Math.max(1, Math.min(50, Number(e.target.value) || 8)) }))}
                onBlur={() => save(cfg)} />
              <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginTop: 6 }}>与扫描预算分开计；确定性扫描不再饿死深度提议。</span>
            </div>
          )}
          <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 10, marginTop: 4 }}>
            <button className="btn" disabled={running} onClick={runNow}>
              <Icon name="refresh" size={14} style={{ animation: running ? 'spin 1s linear infinite' : undefined }} />{running ? '运行中…' : '立即跑一轮'}
            </button>
            {(busy || status) && <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{busy ? '保存中…' : status}</span>}
          </div>
        </div>
      </div>
    </div>
  );
}

function ToolsSettings() {
  const [ws, setWs] = useState<WebSearchSettings>({ provider: '', endpoint: '', max_results: 5, api_key_set: false, fetch_content: false });
  const [apiKey, setApiKey] = useState('');
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState('');
  const [open, setOpen] = useState(false); // 默认折叠，节省高度

  useEffect(() => { getWebSearchSettings().then(setWs).catch(e => setStatus(String(e))); }, []);

  // 生效 provider：未配置/配置不全一律退回免 Key 的 DuckDuckGo，故默认始终可用。
  const eff = ws.provider || 'duckduckgo';
  const enabled =
    eff === 'duckduckgo' ||
    (eff === 'tavily' && (ws.api_key_set || apiKey.trim().length > 0)) ||
    (eff === 'searxng' && ws.endpoint.trim().length > 0);

  const save = async () => {
    setBusy(true);
    try {
      const r = await setWebSearchSettings(ws.provider || 'duckduckgo', ws.endpoint, ws.max_results, apiKey.trim() || undefined, ws.fetch_content);
      setWs(r); setApiKey(''); setStatus('已保存');
    } catch (e) { setStatus(String(e)); }
    finally { setBusy(false); }
  };

  return (
    <div className="set-inner rise">
      <div className="set-h">工具 & MCP</div>
      <div className="set-desc">为 Agent 启用外部工具。工具结果视为不可信外部输入，回灌上下文前自动过安全过滤。仅 OpenAI 兼容 / Anthropic 接口规范的 LLM 支持工具调用。</div>

      <div className="cfg-card" style={{ borderColor: enabled ? 'var(--ember-tint-strong)' : undefined }}>
        <div className="cfg-top" style={{ gap: 10, cursor: 'pointer', userSelect: 'none' }} onClick={() => setOpen(o => !o)}>
          <div className="cfg-logo" style={{ background: 'var(--ember)', width: 28, height: 28 }}><Icon name="search" size={15} /></div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div className="cfg-name cfg-name-line"><span className="cfg-name-text">联网搜索 · web_search</span>
              <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginLeft: 6 }}>TOOL</span>
            </div>
            <div className="cfg-sub">原生免 Key 联网搜索（DuckDuckGo），可选搜索后自动读取正文</div>
          </div>
          <span className={'chip ' + (enabled ? 'green' : 'amber')} style={{ flexShrink: 0 }}>{enabled ? '已启用' : '未启用'}</span>
          <Icon name={open ? 'chevDown' : 'chevRight'} size={18} style={{ color: 'var(--text-3)', flexShrink: 0 }} />
        </div>

        {open && (
        <div className="cfg-fields rise" style={{ marginTop: 14 }}>
          <div className="field"><label>搜索 Provider</label>
            <Select value={ws.provider || 'duckduckgo'} onChange={val => setWs(w => ({ ...w, provider: val }))}
              options={[
                { value: 'duckduckgo', label: 'DuckDuckGo（免 Key · 默认）' },
                { value: 'searxng', label: 'SearXNG（自托管，无需 Key）' },
                { value: 'tavily', label: 'Tavily（需 API Key）' },
              ]} />
          </div>
          <div className="field"><label>返回结果数</label>
            <input type="number" min="1" max="10" value={ws.max_results}
              onChange={e => setWs(w => ({ ...w, max_results: Math.max(1, Math.min(10, Number(e.target.value) || 5)) }))} />
          </div>
          {ws.provider === 'searxng' && (
            <div className="field full"><label>SearXNG Endpoint</label>
              <input type="text" className="mono" value={ws.endpoint} placeholder="https://searx.example.com"
                onChange={e => setWs(w => ({ ...w, endpoint: e.target.value }))} />
            </div>
          )}
          {ws.provider === 'tavily' && (
            <div className="field full"><label><Icon name="key" size={11} style={{ verticalAlign: -1, marginRight: 4 }} />Tavily API Key</label>
              <input type="password" className="mono" value={apiKey}
                placeholder={ws.api_key_set ? '已设置（留空则不修改）' : 'tvly-...'}
                onChange={e => setApiKey(e.target.value)} />
            </div>
          )}
          <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
            <button type="button" className={`switch${ws.fetch_content ? ' on' : ''}`} role="switch" aria-checked={ws.fetch_content}
              onClick={() => setWs(w => ({ ...w, fetch_content: !w.fetch_content }))}><i /></button>
            <div style={{ minWidth: 0 }}>
              <div style={{ fontWeight: 600, fontSize: 'var(--text-control)' }}>搜索后自动读取正文</div>
              <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>命中后自动抓取前几条结果的正文摘录一并回灌（更慢但更省往返；Agent 也可按需用 web_fetch 抓取单个链接）</div>
            </div>
          </div>
          <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
            <button className="btn btn-primary" disabled={busy} onClick={save}><Icon name="check" size={14} />保存工具配置</button>
            {status && <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{status}</span>}
          </div>
          <div className="field full">
            <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>
              默认走 DuckDuckGo，无需任何配置即可联网搜索。需在「角色 Agent / 自定义 Agent」能力中勾选 web_search / web_fetch 的 Agent 才会实际调用对应工具。
            </span>
          </div>
        </div>
        )}
      </div>

      <OpenDesignSettingsCard />

      <McpServers />
    </div>
  );
}

// OpenDesign 本地服务：默认自动模式（检测/克隆 nexu-io/open-design → 安装 → tools-dev 启动）。
function OpenDesignSettingsCard() {
  const [od, setOd] = useState<OpenDesignSettings>({ command: '', url: '', repo_path: '' });
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState('');
  const [log, setLog] = useState('');
  const [open, setOpen] = useState(false); // 默认折叠，节省高度

  useEffect(() => { getOpenDesignSettings().then(setOd).catch(e => setStatus(String(e))); }, []);

  const save = async () => {
    setBusy(true);
    try {
      const r = await setOpenDesignSettings(od.command, od.url, od.repo_path);
      setOd(r); setStatus('已保存');
    } catch (e) { setStatus(String(e)); }
    finally { setBusy(false); }
  };

  const viewLog = async () => {
    try { setLog((await getOpenDesignLog()) || '（暂无日志：尚未触发过自动启动）'); }
    catch (e) { setLog(String(e)); }
  };

  return (
    <div className="cfg-card" style={{ marginTop: 14 }}>
      <div className="cfg-top" style={{ gap: 10, cursor: 'pointer', userSelect: 'none' }} onClick={() => setOpen(o => !o)}>
        <div className="cfg-logo" style={{ background: 'var(--violet)', width: 28, height: 28 }}><Icon name="palette" size={15} /></div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div className="cfg-name cfg-name-line"><span className="cfg-name-text">OpenDesign · 本地服务</span>
            <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginLeft: 6 }}>DESIGN</span>
          </div>
          <div className="cfg-sub">默认自动拉起：检测本地 open-design 检出（无则克隆）→ pnpm install → tools-dev 启动，就绪后打开浏览器</div>
        </div>
        <Icon name={open ? 'chevDown' : 'chevRight'} size={18} style={{ color: 'var(--text-3)', flexShrink: 0 }} />
      </div>

      {open && (
      <div className="cfg-fields rise" style={{ marginTop: 14 }}>
        <div className="field full"><label>本地检出路径（可选）</label>
          <input type="text" className="mono" value={od.repo_path} placeholder="留空则自动探测 ~/projects/open-design 等，找不到再克隆到自管目录"
            onChange={e => setOd(o => ({ ...o, repo_path: e.target.value }))} />
        </div>
        <div className="field full"><label>自定义启动命令（高级 · 留空走自动模式）</label>
          <input type="text" className="mono" value={od.command} placeholder="留空＝自动模式；填写则改为执行该命令并打开下方 URL"
            onChange={e => setOd(o => ({ ...o, command: e.target.value }))} />
        </div>
        <div className="field full"><label>访问 URL（仅自定义模式需要）</label>
          <input type="text" className="mono" value={od.url} placeholder="自动模式下由 tools-dev 解析，无需填写"
            onChange={e => setOd(o => ({ ...o, url: e.target.value }))} />
        </div>
        <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
          <button className="btn btn-primary" disabled={busy} onClick={save}><Icon name="check" size={14} />保存 OpenDesign 配置</button>
          <button className="btn" onClick={viewLog}><Icon name="file" size={14} />查看启动日志</button>
          {status && <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{status}</span>}
        </div>
        {log && (
          <div className="field full">
            <pre className="mono" style={{ maxHeight: 240, overflow: 'auto', background: 'var(--code-bg)', padding: 10, borderRadius: 'var(--radius-sm)', fontSize: 'var(--text-caption)', color: 'var(--text-2)', whiteSpace: 'pre-wrap', wordBreak: 'break-all' }}>{log}</pre>
          </div>
        )}
        <div className="field full">
          <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>
            自动模式需要本机有 Node 24 + pnpm（建议 corepack）。首次会克隆并 pnpm install（较慢），随后用 `pnpm tools-dev start web` 启动并轮询就绪。失败时点「查看启动日志」排查。
          </span>
        </div>
      </div>
      )}
    </div>
  );
}

// 代码情报默认能力映射（codegraph）。「填入默认」按钮用，也是占位提示。
const CODEGRAPH_CAP_PRESET = JSON.stringify({
  locate_symbol: { tool: 'codegraph_search', args: { query: '$SYMBOL', projectPath: '$REPO', limit: 1 } },
  find_callers: { tool: 'codegraph_callers', args: { symbol: '$SYMBOL', projectPath: '$REPO', limit: 5 } },
  impact_analysis: { tool: 'codegraph_impact', args: { symbol: '$SYMBOL', projectPath: '$REPO', depth: 2 } },
}, null, 2);

function McpServers() {
  const [servers, setServers] = useState<McpServer[]>([]);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [exp, setExp] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, McpServerInput>>({});
  const [test, setTest] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [confirmDel, setConfirmDel] = useState<string | null>(null);
  // 代码情报「高级设置（能力映射）」的展开态——默认折叠，留空即走约定，无需展开。
  const [advExp, setAdvExp] = useState<Set<string>>(new Set());
  const toggleAdv = (id: string) => setAdvExp(prev => {
    const n = new Set(prev); n.has(id) ? n.delete(id) : n.add(id); return n;
  });

  const reload = () => Promise.all([listMcpServers(), listAgents()])
    .then(([s, a]) => { setServers(s); setAgents(a); })
    .catch(() => {});
  useEffect(() => { reload(); }, []);

  const setDraft = (id: string, field: keyof McpServerInput, val: unknown) =>
    setDrafts(d => ({ ...d, [id]: { ...d[id], [field]: val } }));

  const val = <K extends keyof McpServer>(s: McpServer, d: McpServerInput, f: K): McpServer[K] =>
    (d[f as keyof McpServerInput] as McpServer[K] | undefined) ?? s[f];

  const scopedAgents = (s: McpServer, d: McpServerInput): string[] => {
    try { return JSON.parse(String(val(s, d, 'agent_ids_json') ?? '[]')); } catch { return []; }
  };
  const toggleAgent = (s: McpServer, d: McpServerInput, agentId: string) => {
    const cur = new Set(scopedAgents(s, d));
    if (cur.has(agentId)) cur.delete(agentId); else cur.add(agentId);
    setDraft(s.id, 'agent_ids_json', JSON.stringify([...cur]));
  };

  // 能力映射是否「未填写」（空 / {} / 纯空白）。
  const mapIsEmpty = (m?: string | null) => !m || ['', '{}'].includes(m.trim());

  const save = async (id: string) => {
    const d = drafts[id] ?? {};
    if (Object.keys(d).length === 0) { setExp(null); return; }
    setBusy(id);
    try {
      let updated = await updateMcpServer(id, d);
      // 适用于编码 Agent 且能力映射未填写：保存后按约定发现并回填持久化（用刚存的最新配置连接）。
      if (updated.for_code_agent && mapIsEmpty(updated.capability_map_json)) {
        const m = await discoverCodeIntelMap(id).catch(() => '{}');
        if (!mapIsEmpty(m)) {
          updated = await updateMcpServer(id, { capability_map_json: m });
          setAdvExp(prev => new Set(prev).add(id));
          setTest(t => ({ ...t, [id]: '✓ 已按约定发现并填入能力映射' }));
        }
      }
      setServers(ss => ss.map(s => s.id === id ? updated : s));
      setDrafts(x => { const n = { ...x }; delete n[id]; return n; });
    } catch (e) { setTest(t => ({ ...t, [id]: '保存失败: ' + String(e) })); }
    finally { setBusy(null); }
  };
  const addNew = async () => {
    const s = await createMcpServer({ name: '新 MCP Server', transport: 'stdio' });
    setServers(ss => [...ss, s]); setExp(s.id);
  };
  const doDelete = async (id: string) => {
    await deleteMcpServer(id);
    setServers(ss => ss.filter(s => s.id !== id));
    setConfirmDel(null);
  };
  const runTest = async (id: string) => {
    setBusy(id); setTest(t => ({ ...t, [id]: '连接中…' }));
    try {
      const tools = await testMcpConnection(id);
      setTest(t => ({ ...t, [id]: tools.length ? `✓ 发现 ${tools.length} 个工具：${tools.join(', ')}` : '✓ 已连接，但无工具' }));
      // 适用于编码 Agent 且能力映射未填写：按约定发现并填入草稿（供检查后保存）。
      const sv = servers.find(x => x.id === id); const dr = drafts[id] ?? {};
      const forCodeNow = Boolean(dr.for_code_agent ?? sv?.for_code_agent ?? false);
      const curMap = String(dr.capability_map_json ?? sv?.capability_map_json ?? '');
      if (forCodeNow && mapIsEmpty(curMap)) {
        const m = await discoverCodeIntelMap(id).catch(() => '{}');
        if (!mapIsEmpty(m)) {
          setDraft(id, 'capability_map_json', m);
          setAdvExp(prev => new Set(prev).add(id));
          setTest(t => ({ ...t, [id]: (t[id] ?? '') + '　·　已按约定填入能力映射，检查后保存生效' }));
        }
      }
    } catch (e) { setTest(t => ({ ...t, [id]: '✗ ' + String(e) })); }
    finally { setBusy(null); }
  };

  return (
    <div style={{ marginTop: 22 }}>
      {confirmDel && <ConfirmModal msg="确认删除此 MCP Server？" onOk={() => doDelete(confirmDel)} onCancel={() => setConfirmDel(null)} />}
      <div className="set-h" style={{ fontSize: 'var(--text-section)' }}>MCP Servers</div>
      <div className="set-desc">接入外部 MCP 工具生态。每个 server 的适用面有两个正交维度，可同时开启：勾选「角色 Agent」让会议室 Agent 调用其工具；开「适用于编码 Agent」让编码 Agent 在 worktree 内实时调用本 server 工具（pull），并对含代码情报能力的 server 额外做执行前预查（push）。MCP 工具结果同样过安全过滤。</div>

      {servers.map(s => {
        const d = drafts[s.id] ?? {};
        const transport = String(val(s, d, 'transport') ?? 'stdio') as McpTransport;
        const forCode = Boolean(val(s, d, 'for_code_agent') ?? false);
        const scoped = scopedAgents(s, d);
        const en = Boolean(d.enabled ?? s.enabled);
        return (
          <div className="cfg-card" key={s.id} style={{ padding: exp === s.id ? '13px 16px' : '8px 12px', marginBottom: 6, ...(exp === s.id ? { borderColor: 'var(--ember-tint-strong)' } : {}) }}>
            <div className="cfg-top" onClick={() => setExp(exp === s.id ? null : s.id)} style={{ cursor: 'pointer', gap: 10 }}>
              <div className="cfg-logo" style={{ background: 'var(--blue, #4f8ed1)', width: 28, height: 28 }}><Icon name="zap" size={15} /></div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div className="cfg-name cfg-name-line"><span className="cfg-name-text">{String(val(s, d, 'name') ?? '')}</span>
                  <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginLeft: 6 }}>{transport.toUpperCase()}</span>
                </div>
                <div className="cfg-sub">{[scoped.length ? `${scoped.length} 个角色 Agent` : '', forCode ? '编码 Agent' : ''].filter(Boolean).join(' · ') || '未启用任何适用面'}</div>
              </div>
              <span className={'chip ' + (en ? 'green' : 'amber')} style={{ flexShrink: 0 }}>{en ? '已启用' : '未启用'}</span>
              <Icon name={exp === s.id ? 'chevDown' : 'chevRight'} size={18} style={{ color: 'var(--text-3)' }} />
            </div>
            {exp === s.id && (
              <div className="cfg-fields rise" style={{ marginTop: 14 }}>
                <div className="field"><label>名称</label>
                  <input value={String(val(s, d, 'name') ?? '')} onChange={e => setDraft(s.id, 'name', e.target.value)} />
                </div>
                <div className="field"><label>传输方式</label>
                  <Select value={transport} onChange={v => setDraft(s.id, 'transport', v)}
                    options={[{ value: 'stdio', label: 'stdio（本地子进程）' }, { value: 'http', label: 'HTTP（远程 streamable）' }]} />
                </div>
                <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
                  <Switch on={forCode} onToggle={() => setDraft(s.id, 'for_code_agent', !forCode)} />
                  <div style={{ flex: 1 }}>
                    <div style={{ fontSize: 'var(--text-control)', color: 'var(--text-2)' }}>适用于编码 Agent</div>
                    <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>
                      开启后本 server 对编码 Agent 生效，两条机制：①<b>实时调用（pull）</b>——工具注入 CLI，agent 在 worktree 内自主调用任意 MCP 工具（claude 完整支持 / codex 实验性 / opencode 暂不支持）；②<b>代码情报预查（push）</b>——若含 locate_symbol 等能力，执行前预查并注入 prompt。与下方「角色 Agent」正交，可同时开。
                    </div>
                  </div>
                </div>
                {transport === 'stdio' ? (<>
                  <div className="field full"><label>启动命令 command</label>
                    <input className="mono" value={String(val(s, d, 'command') ?? '')} placeholder="npx / uvx / node …"
                      onChange={e => setDraft(s.id, 'command', e.target.value)} />
                  </div>
                  <div className="field full"><label>参数 args（JSON 数组）</label>
                    <input className="mono" value={String(val(s, d, 'args_json') ?? '[]')} placeholder='["-y","@modelcontextprotocol/server-filesystem","/path"]'
                      onChange={e => setDraft(s.id, 'args_json', e.target.value)} />
                  </div>
                  <div className="field full"><label>环境变量 env（JSON 对象，密钥留空则不改）</label>
                    <input className="mono" value={String(val(s, d, 'env_json') ?? '{}')} placeholder='{"API_KEY":""}'
                      onChange={e => setDraft(s.id, 'env_json', e.target.value)} />
                  </div>
                </>) : (<>
                  <div className="field full"><label>服务 URL</label>
                    <input className="mono" value={String(val(s, d, 'url') ?? '')} placeholder="https://host/mcp"
                      onChange={e => setDraft(s.id, 'url', e.target.value)} />
                  </div>
                  <div className="field full"><label>请求头 headers（JSON 对象，密钥留空则不改）</label>
                    <input className="mono" value={String(val(s, d, 'headers_json') ?? '{}')} placeholder='{"Authorization":"Bearer "}'
                      onChange={e => setDraft(s.id, 'headers_json', e.target.value)} />
                  </div>
                </>)}
                {forCode && (
                  <div className="field full">
                    <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>
                      编码 Agent 的代码情报由 AutoForge 在执行前 push 式调用并注入实现 prompt，三家 code agent 一致受益。
                      默认按工具命名自动发现（codegraph 等常见工具无需任何配置）；仅非常规工具才需在下方手动指定映射。
                    </span>
                    <button type="button" className="btn btn-sm btn-ghost" style={{ alignSelf: 'flex-start', marginTop: 6 }}
                      onClick={() => toggleAdv(s.id)}>
                      <Icon name={advExp.has(s.id) ? 'chevDown' : 'chevRight'} size={13} />高级：自定义能力映射
                    </button>
                    {advExp.has(s.id) && (
                      <div style={{ marginTop: 8 }}>
                        <label style={{ marginBottom: 8 }}>能力映射 capability_map（可选，留空自动按工具命名约定发现；占位符 $SYMBOL / $REPO）</label>
                        <textarea className="mono" rows={6} style={{ resize: 'vertical', width: '100%' }}
                          value={String(val(s, d, 'capability_map_json') ?? '{}')}
                          placeholder={`格式示例（仅 codegraph）：\n${CODEGRAPH_CAP_PRESET}`}
                          onChange={e => setDraft(s.id, 'capability_map_json', e.target.value)} />
                        <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginTop: 4 }}>
                          locate_symbol / find_callers → 各自的工具名 + 参数。$SYMBOL 替换为符号名、$REPO 替换为主仓路径。
                        </span>
                      </div>
                    )}
                  </div>
                )}
                <div className="field full"><label>适用的角色 Agent（勾选后该会议室 Agent 加载本 server 的工具，pull 式直接调用）</label>
                  <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, padding: '6px 0' }}>
                    {agents.length === 0 && <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>暂无 Agent</span>}
                    {agents.map(a => {
                      const on = scoped.includes(a.id);
                      return (
                        <button key={a.id} className={'filter-chip' + (on ? ' on' : '')}
                          onClick={() => toggleAgent(s, d, a.id)}>
                          {on ? '✓ ' : ''}{a.name}
                        </button>
                      );
                    })}
                  </div>
                </div>
                <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 12, paddingTop: 4 }}>
                  <Switch on={en} onToggle={() => setDraft(s.id, 'enabled', !en)} />
                  <span style={{ fontSize: 'var(--text-control)', color: 'var(--text-2)', flex: 1 }}>启用此 server</span>
                  <button className="btn btn-sm btn-danger" onClick={() => setConfirmDel(s.id)}><Icon name="trash" size={13} />删除</button>
                  <button className="btn btn-sm" disabled={busy === s.id} onClick={() => runTest(s.id)}><Icon name="zap" size={13} />测试连接</button>
                  <button className="btn btn-sm btn-primary" disabled={busy === s.id} onClick={() => save(s.id)}><Icon name="check" size={13} />保存</button>
                </div>
                {test[s.id] && (
                  <div className="field full"><span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)', wordBreak: 'break-all' }}>{test[s.id]}</span></div>
                )}
              </div>
            )}
          </div>
        );
      })}
      <div className="cfg-card add" onClick={addNew} style={{ padding: 12, marginBottom: 0 }}><Icon name="plus" size={16} />新增 MCP Server</div>
    </div>
  );
}

function CustomAgents({ onChanged }: { onChanged: () => void }) {
  const builtinTools = useBuiltinTools();
  const [agents, setAgents] = useState<Agent[]>([]);
  const [hideIds, setHideIds] = useState<Set<string>>(new Set());
  const [llmNames, setLlmNames] = useState<LlmRef[]>([]);
  const [exp, setExp] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, Partial<Agent>>>({});
  const [saveStatus, setSaveStatus] = useState<Record<string, string>>({});
  const [confirmDel, setConfirmDel] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const reload = () =>
    Promise.all([listAgents(), listLlmConfigs(), listRoleCatalog()]).then(([ags, llms, cat]) => {
      setAgents(ags);
      setHideIds(new Set(cat.map(s => s.holder?.id).filter(Boolean) as string[]));
      setLlmNames(llms.map(l => ({ id: l.id, name: l.name, enabled: l.enabled })));
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
      void refreshAgents();  // 同步全局 store：名称/可见性变更即时反映到 @提及与头像
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
    void refreshAgents();
    setConfirmDel(null);
    onChanged();
  };

  const addNew = async () => {
    const a = await createAgent({ name: '新对话角色', system_prompt: AGENT_TEMPLATES[0].prompt, prompt_mode: 'custom', role_type: 'business' });
    setAgents(as => [...as, a]);
    void refreshAgents();
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
              {(() => {
                const st = llmBindingState(a.llm_id, llmNames);
                if (!a.enabled || st === 'ok' || st === 'none') return null;
                return <span className="chip red" style={{ flexShrink: 0, fontSize: 'var(--text-micro)', padding: '1px 6px' }}
                  title={st === 'disabled' ? '绑定的 LLM 已停用，运行将失败' : '绑定的 LLM 配置已删除，运行将失败'}>
                  <Icon name="alert" size={11} />{st === 'disabled' ? 'LLM 已停用' : 'LLM 缺失'}</span>;
              })()}
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
                      options={[{ value: '', label: '— 未指定 —' }, ...llmNames.map(l => ({ value: l.id, label: llmOptionLabel(l) }))]} />
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
                    <label>工具能力</label>
                    <div style={{ display: 'flex', gap: 18, alignItems: 'center', flexWrap: 'wrap', padding: '8px 0' }}>
                      {(() => {
                        const cap = d.capabilities_json !== undefined ? String(d.capabilities_json ?? '') : a.capabilities_json;
                        return builtinTools.map(t => {
                          const on = agentHasTool(cap, t.name);
                          return (
                            <label key={t.name} style={{ display: 'inline-flex', alignItems: 'center', gap: 8, color: 'var(--text-2)', fontSize: 'var(--text-control)' }}
                              title={t.needs_project ? '需对话/任务绑定项目才生效' : undefined}>
                              <Switch on={on} onToggle={() => setDraft(a.id, 'capabilities_json', toggleAgentTool(cap, t.name, !on))} />{t.label} {t.name}
                            </label>
                          );
                        });
                      })()}
                      <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>工具在任何调用该 Agent LLM 的场景下都可用，由 LLM 自行决定是否调用（agent loop）。标注「需项目」的工具仅在关联了项目时装配；联网搜索需先在「工具 & MCP」配置 Provider。均仅 OpenAI/Anthropic 规范的 LLM 生效。</span>
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
  llms: LlmRef[];
  onApply: (kind: string, payload: Parameters<typeof setRoleSlot>[1]) => Promise<void>;
}) {
  const h = slot.holder;
  const mode = (h?.prompt_mode ?? 'builtin') as 'builtin' | 'append' | 'custom';
  const [supplement, setSupplement] = useState(h?.system_prompt ?? '');
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState(false);
  const dirty = supplement !== (h?.system_prompt ?? '');
  const builtinTools = useBuiltinTools();

  const apply = async (payload: Parameters<typeof setRoleSlot>[1]) => {
    setBusy(true);
    try { await onApply(slot.kind, payload); } finally { setBusy(false); }
  };

  // 工具能力开关（所有系统角色通用，含 llm_only 角色）。开关从后端内置工具目录动态渲染。
  // 运行时由后端按 capabilities 白名单 + 上下文决定是否真正加载；未开启则与原行为一致。
  const toolCaps = () => {
    const cap = h?.capabilities_json;
    return (
      <div className="field full">
        <label>工具能力</label>
        <div style={{ display: 'flex', gap: 18, alignItems: 'center', flexWrap: 'wrap', padding: '8px 0' }}>
          {builtinTools.map(t => {
            const on = agentHasTool(cap, t.name);
            return (
              <label key={t.name} style={{ display: 'inline-flex', alignItems: 'center', gap: 8, color: 'var(--text-2)', fontSize: 'var(--text-control)' }}
                title={t.needs_project ? '需对话/任务绑定项目才生效' : undefined}>
                <Switch on={on} onToggle={() => apply({ capabilities_json: toggleAgentTool(cap, t.name, !on) })} />{t.label} {t.name}
              </label>
            );
          })}
        </div>
        <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>工具在任何调用该角色 LLM 的场景下都可用，由 LLM 自行决定是否调用（agent loop）。标注「需项目」的工具仅在关联了项目时装配；联网搜索需先在「工具 &amp; MCP」配置 Provider。均仅 OpenAI/Anthropic 规范的 LLM 生效。</span>
      </div>
    );
  };

  const llmState = llmBindingState(h?.llm_id, llms);
  const status = !h ? { t: '未配置', c: '' }
    : !h.enabled ? { t: '已停用', c: '' }
    : !h.llm_id ? { t: '缺 LLM', c: 'amber' }
    : llmState === 'disabled' ? { t: 'LLM 已停用', c: 'red' }
    : llmState === 'missing' ? { t: 'LLM 缺失', c: 'red' }
    : { t: '已启用', c: 'green' };
  const boundLlm = h?.llm_id ? llms.find(l => l.id === h.llm_id) : undefined;
  const llmName = h?.llm_id
    ? (boundLlm ? `${boundLlm.name}${boundLlm.enabled ? '' : '（已停用）'}` : `${h.llm_id}（缺失）`)
    : '未指定 LLM';
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
            {open ? slot.desc : (h ? (slot.llm_only ? llmName : `${llmName} · 提示词 ${modeLabel}`) : slot.desc)}
          </div>
        </div>
        {!open && h?.mentionable && <span className="chip" style={{ flexShrink: 0, fontSize: 'var(--text-micro)', padding: '1px 6px' }} title="可拉入群聊">群</span>}
        {!open && h?.visible_in_chat && <span className="chip" style={{ flexShrink: 0, fontSize: 'var(--text-micro)', padding: '1px 6px' }} title="可私聊">私</span>}
        {!open && h?.memory_enabled && <span className="chip ember" style={{ flexShrink: 0, fontSize: 'var(--text-micro)', padding: '1px 6px' }} title="已启用 Innate 记忆召回">记</span>}
        <span className={'chip ' + status.c} style={{ flexShrink: 0 }}>{status.t}</span>
        <Icon name={open ? 'chevDown' : 'chevRight'} size={18} style={{ color: 'var(--text-3)', flexShrink: 0 }} />
      </div>
      {open && slot.usage && (
        <div className="rise" style={{
          display: 'flex', alignItems: 'center', gap: 6, marginTop: 10,
          padding: '4px 9px', borderRadius: 'var(--radius-sm)',
          background: 'var(--ember-tint)', color: 'var(--text-2)',
          fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', lineHeight: 'var(--leading-snug)',
        }} title="该角色 LLM 在产品里被实际调用的位置">
          <Icon name="at" size={12} style={{ color: 'var(--ember)', flexShrink: 0 }} />
          <span style={{ color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '.1em', flexShrink: 0 }}>用于</span>
          <span style={{ minWidth: 0 }}>{slot.usage}</span>
        </div>
      )}
      {open && slot.llm_only && (
      <div className="cfg-fields rise" style={{ marginTop: 14 }}>
        <div className="field full"><label>使用的 LLM</label>
          <Select value={h?.llm_id ?? ''} options={[{ value: '', label: '— 未指定（Innate 回退启发式蒸馏）—' }, ...llms.map(l => ({ value: l.id, label: llmOptionLabel(l) }))]}
            onChange={val => apply({ llm_id: val, enabled: true })} />
          <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>仅支持有 HTTP API 的 LLM（OpenAI 兼容 / Anthropic）；Claude CLI 无法用于 Innate。Innate 已内置进程内，配置即时生效、不写任何全局文件。</span>
        </div>
        <div className="field full">
          <label style={{ display: 'inline-flex', alignItems: 'center', gap: 8, color: 'var(--text-2)', fontSize: 'var(--text-control)' }}>
            <Switch on={Boolean(h?.enabled)} onToggle={() => apply({ enabled: !(h?.enabled) })} />启用
          </label>
        </div>
        {toolCaps()}
      </div>
      )}
      {open && !slot.llm_only && (
      <div className="cfg-fields rise" style={{ marginTop: 14 }}>
        <div className="field"><label>使用的 LLM</label>
          <Select value={h?.llm_id ?? ''} options={[{ value: '', label: '— 未指定 —' }, ...llms.map(l => ({ value: l.id, label: llmOptionLabel(l) }))]}
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
            <label style={{ display: 'inline-flex', alignItems: 'center', gap: 8, color: 'var(--text-2)', fontSize: 'var(--text-control)' }} title="开启后该角色会召回本项目历史经验注入提示词，随使用越来越准（Innate 已内置，无需外部安装）">
              <Switch on={h ? Boolean(h.memory_enabled) : true} onToggle={() => apply({ memory_enabled: !(h?.memory_enabled ?? true) })} />启用记忆
            </label>
          </div>
        </div>
        {toolCaps()}
      </div>
      )}
    </div>
  );
}

const ROLE_GROUPS: { id: RoleSlot['group']; title: string; icon: string; color: string; sub: string }[] = [
  { id: 'orchestration', title: '群聊编排角色', icon: 'bot',     color: 'var(--blue)',  sub: '会议室多 Agent 协作的内置职责' },
  { id: 'delivery',      title: '交付与项目角色', icon: 'package', color: 'var(--green)', sub: '交付流水线与项目工具的 AI 职责' },
  { id: 'pipeline',      title: '需求流水线角色', icon: 'sliders', color: 'var(--ember)', sub: '分析 / 测试阶段' },
  { id: 'knowledge',     title: '知识层（Innate）', icon: 'brain', color: 'var(--ember)', sub: 'Innate 自成长用的蒸馏 LLM 与 Embedding 模型' },
];

// Innate embedding 模型配置卡（recall 语义检索用；非聊天 LLM，独立于 llm_configs）。
function EmbeddingConfigCard() {
  const [form, setForm] = useState<EmbeddingSettings>({ provider: 'openai', base_url: '', model_id: '', api_key: '', dim: 1536 });
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState('');

  useEffect(() => { getKnowledgeEmbedding().then(setForm).catch(e => setStatus(String(e))); }, []);

  const save = async () => {
    setBusy(true);
    try { setForm(await setKnowledgeEmbedding(form)); setStatus('已保存 · 已同步至 Innate'); }
    catch (e) { setStatus(String(e)); }
    finally { setBusy(false); }
  };

  const configured = form.model_id.trim().length > 0;
  return (
    <div className="cfg-card" style={{ padding: open ? '13px 16px' : '8px 12px', marginBottom: 0, ...(open ? { borderColor: 'var(--ember-tint-strong)' } : {}) }}>
      <div className="cfg-top" onClick={() => setOpen(o => !o)} style={{ cursor: 'pointer', gap: 10 }}>
        <div className="cfg-logo" style={{ background: 'var(--ember)', width: 28, height: 28 }}><Icon name="layers" size={15} /></div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div className="cfg-name cfg-name-line"><span className="cfg-name-text">Embedding 模型</span>
            <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginLeft: 6 }}>Embedding</span>
          </div>
          <div className="cfg-sub" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {open ? '语义召回（innate recall）用的向量模型' : (configured ? `${form.model_id} · dim ${form.dim}` : '语义召回向量模型（未配置则用哈希占位）')}
          </div>
        </div>
        <span className={'chip ' + (configured ? 'green' : 'amber')} style={{ flexShrink: 0 }}>{configured ? '已配置' : '未配置'}</span>
        <Icon name={open ? 'chevDown' : 'chevRight'} size={18} style={{ color: 'var(--text-3)', flexShrink: 0 }} />
      </div>
      {open && (
        <div className="cfg-fields rise" style={{ marginTop: 14 }}>
          <div className="field"><label>Provider</label>
            <Select value={form.provider || 'openai'} options={[{ value: 'openai', label: 'openai（兼容）' }]}
              onChange={val => setForm(f => ({ ...f, provider: val }))} />
            <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>Anthropic 无 embedding API，仅 OpenAI 兼容端点。</span>
          </div>
          <div className="field"><label>向量维度 dim</label>
            <input type="number" min="1" max="8192" value={form.dim}
              onChange={e => setForm(f => ({ ...f, dim: Number(e.target.value) }))} />
            <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>须与模型实际产出一致（如 text-embedding-v4=1024）。</span>
          </div>
          <div className="field full"><label>Base URL</label>
            <input type="text" value={form.base_url} placeholder="https://dashscope.aliyuncs.com/compatible-mode/v1"
              onChange={e => setForm(f => ({ ...f, base_url: e.target.value }))} />
          </div>
          <div className="field full"><label>Model ID</label>
            <input type="text" value={form.model_id} placeholder="text-embedding-v4 / text-embedding-3-small"
              onChange={e => setForm(f => ({ ...f, model_id: e.target.value }))} />
          </div>
          <div className="field full"><label>API Key</label>
            <input type="password" value={form.api_key} placeholder="embedding API Key"
              onChange={e => setForm(f => ({ ...f, api_key: e.target.value }))} />
          </div>
          <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
            <button className="btn btn-primary" disabled={busy} onClick={save}><Icon name="check" size={14} />保存 Embedding</button>
            {status && <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{status}</span>}
          </div>
        </div>
      )}
    </div>
  );
}

function RoleCardsSection({ onChanged }: { onChanged: () => void }) {
  const [slots, setSlots] = useState<RoleSlot[]>([]);
  const [llms, setLlms] = useState<LlmRef[]>([]);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState('');
  const [openGroups, setOpenGroups] = useState<Record<string, boolean>>({}); // 默认收起

  useEffect(() => {
    Promise.all([listRoleCatalog(), listLlmConfigs()]).then(([s, l]) => {
      setSlots(s); setLlms(l.map(x => ({ id: x.id, name: x.name, enabled: x.enabled }))); setLoading(false);
    }).catch(e => { setErr(String(e)); setLoading(false); });
  }, []);

  const apply = async (kind: string, payload: Parameters<typeof setRoleSlot>[1]) => {
    try { setSlots(await setRoleSlot(kind, payload)); void refreshAgents(); onChanged(); }
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
        // 完整配置 = 有持有 Agent + 已启用 + 绑定的 LLM 本身可用（停用/缺失均不计入）
        const active = rows.filter(r => r.holder?.enabled && llmBindingState(r.holder?.llm_id, llms) === 'ok').length;
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
                {g.id === 'knowledge' && <EmbeddingConfigCard />}
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

interface CodeAgentDraft { kind: string; label: string; program: string; model: string; fast_model: string; strong_model: string; extra: string; enabled: boolean; }
const CODE_AGENT_KINDS = [
  { value: 'claude', label: 'claude（Claude Code）' },
  { value: 'codex', label: 'codex（Codex CLI）' },
  { value: 'opencode', label: 'opencode' },
];

function CodeAgentSettings() {
  const [agents, setAgents] = useState<CodeAgentT[]>([]);
  const [drafts, setDrafts] = useState<Record<string, CodeAgentDraft>>({});
  // 'loading' = 探测中；CodeAgentProbe = 探测结果；null = 调用失败；undefined = 未检测。
  const [auth, setAuth] = useState<Record<string, CodeAgentProbe | null | 'loading'>>({});
  const [loading, setLoading] = useState(true);
  const [msg, setMsg] = useState('');
  // 默认全部折叠，只显示头部摘要；点击头部展开编辑。
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const toggleExpand = (id: string) => setExpanded(prev => {
    const next = new Set(prev);
    next.has(id) ? next.delete(id) : next.add(id);
    return next;
  });

  const toDraft = (a: CodeAgentT): CodeAgentDraft => {
    let extra: string[] = [];
    try { extra = JSON.parse(a.extra_args_json || '[]'); } catch { extra = []; }
    return { kind: a.kind, label: a.label, program: a.program, model: a.model ?? '', fast_model: a.fast_model ?? '', strong_model: a.strong_model ?? '', extra: extra.join(' '), enabled: a.enabled };
  };

  const load = (autoCheck = false) => {
    setLoading(true);
    listCodeAgents()
      .then(list => {
        setAgents(list);
        setDrafts(Object.fromEntries(list.map(a => [a.id, toDraft(a)])));
        // 进入页面自动检测每个 agent 的可用性，避免一直停在「未检测」。
        // 自动检测只验证工具（轻量、不烧 token）；模型探测留给手动「检测可用性」按钮。
        // 检测命令走进程组隔离（detach_process_group），可安全在此调用。
        if (autoCheck) list.forEach(a => checkAuth(a.id, false));
      })
      .catch(() => setAgents([]))
      .finally(() => setLoading(false));
  };
  useEffect(() => { load(true); }, []);

  const defaultId = agents.find(a => a.is_default)?.id ?? '';
  const enabledOptions = agents.filter(a => a.enabled).map(a => ({ value: a.id, label: `${a.label}（${a.kind}）` }));

  const setDraft = (id: string, patch: Partial<CodeAgentDraft>) =>
    setDrafts(d => ({ ...d, [id]: { ...d[id], ...patch } }));

  const save = async (a: CodeAgentT) => {
    const d = drafts[a.id];
    if (!d) return;
    setMsg('');
    try {
      await upsertCodeAgent({
        id: a.id, kind: d.kind, label: d.label.trim() || d.kind,
        program: d.program.trim() || d.kind, model: d.model.trim() || null,
        fast_model: d.fast_model.trim() || null, strong_model: d.strong_model.trim() || null,
        extra_args: d.extra.split(/\s+/).filter(Boolean), enabled: d.enabled,
      });
      setMsg(`已保存 ${d.label || a.kind}`);
      load();
    } catch (e) { setMsg(String(e)); }
  };

  const makeDefault = async (id: string) => { await setDefaultCodeAgent(id); load(); };

  const remove = async (a: CodeAgentT) => {
    if (!confirm(`删除代码 Agent「${a.label}」？引用它的项目将回落全局默认。`)) return;
    try { await deleteCodeAgent(a.id); load(); } catch (e) { setMsg(String(e)); }
  };

  // probeModel=true 时额外用配置的模型发极小 prompt 验证模型本身可用（慢几秒）；
  // 自动检测传 false 只验证工具。
  const checkAuth = async (id: string, probeModel = false) => {
    setAuth(s => ({ ...s, [id]: 'loading' }));
    try { const r = await checkCodeAgentAuth(id, probeModel); setAuth(s => ({ ...s, [id]: r })); }
    catch { setAuth(s => ({ ...s, [id]: null })); }
  };

  const addCustom = async () => {
    setMsg('');
    try {
      const created = await upsertCodeAgent({ kind: 'claude', label: '自定义 Agent', program: 'claude', enabled: true });
      setExpanded(prev => new Set(prev).add(created.id)); // 新建即展开，便于立刻编辑
      load();
    } catch (e) { setMsg(String(e)); }
  };

  return (
    <div className="set-inner rise">
      <div className="set-h">代码 Agent</div>
      <div className="set-desc">
        选择驱动「代码实现」与「AI 解冲突」的 CLI 编码 agent。所有 agent 都在隔离 worktree 内执行，
        并被统一禁止 remote git 操作。项目可在「项目管理」单独覆盖；未覆盖时跟随这里的全局默认。
        配了「快模型 / 强模型」后，系统会按分析阶段的风险（影响半径 / 文件数 / 复杂度）自动挑选：
        低风险小改动走快模型省时省钱，跨模块或复杂改动走强模型保质量；留空则一律用上方默认模型。
      </div>

      <div className="cfg-card" style={{ marginBottom: 16 }}>
        <div className="field full">
          <label>全局默认代码 Agent</label>
          <Select value={defaultId} onChange={makeDefault} options={enabledOptions}
            placeholder={loading ? '加载中…' : '选择默认 agent'} />
        </div>
      </div>

      {loading ? (
        <div className="empty"><Icon name="code" /><div>加载中…</div></div>
      ) : agents.map(a => {
        const d = drafts[a.id]; if (!d) return null;
        const st = auth[a.id];
        const loadingA = st === 'loading';
        const probe = st && st !== 'loading' ? st : null; // CodeAgentProbe | null
        const tool = probe?.tool;
        const dotColor = tool === true ? 'var(--green)' : (tool === false || st === null) ? 'var(--red)' : 'var(--text-faint)';
        const authText = loadingA ? '检测中…' : tool === true ? '可用' : tool === false ? '未就绪' : st === null ? '检测失败' : '未检测';
        const isOpen = expanded.has(a.id);
        return (
          <div className="panel" key={a.id} style={{ marginBottom: 12 }}>
            <div className="panel-head" style={{ display: 'flex', alignItems: 'center', gap: 10, cursor: 'pointer', userSelect: 'none' }}
              onClick={() => toggleExpand(a.id)}>
              <Icon name="chevron" size={14} style={{ color: 'var(--text-3)', transform: isOpen ? 'rotate(0deg)' : 'rotate(-90deg)', transition: 'transform .15s' }} />
              <span className="chip ember" style={{ fontFamily: 'var(--font-mono)' }}>{d.kind}</span>
              <span style={{ fontWeight: 600 }}>{d.label || d.kind}</span>
              {a.is_default && <span className="chip green">默认</span>}
              {!isOpen && !d.enabled && <span className="chip" style={{ color: 'var(--text-faint)' }}>已停用</span>}
              <span className="dot" style={{ background: dotColor, marginLeft: 'auto' }} />
              <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{authText}</span>
              {probe?.model != null && (
                <span className={`chip ${probe.model ? 'green' : 'red'}`} title={probe.detail || undefined}>
                  模型 {probe.model ? '可用' : '不可用'}{probe.model_name ? ` · ${probe.model_name}` : ''}
                </span>
              )}
            </div>
            {isOpen && (
            <div className="cfg-fields" style={{ padding: '12px 14px' }}>
              <div className="field"><label>类型（适配逻辑）</label>
                <Select value={d.kind} onChange={val => {
                  // 改类型时，若 program 仍是某个 kind 的默认名（未自定义路径），一并同步，
                  // 避免出现「类型 codex 但程序仍指向 claude」的错配。
                  const synced = ['claude', 'codex', 'opencode', ''].includes(d.program.trim());
                  setDraft(a.id, synced ? { kind: val, program: val } : { kind: val });
                }} options={CODE_AGENT_KINDS} />
              </div>
              <div className="field"><label>显示名</label>
                <input value={d.label} onChange={e => setDraft(a.id, { label: e.target.value })} />
              </div>
              <div className="field"><label>可执行程序（PATH 名或绝对路径）</label>
                <input value={d.program} onChange={e => setDraft(a.id, { program: e.target.value })} placeholder={d.kind} />
              </div>
              <div className="field"><label>模型（可空，{d.kind === 'opencode' ? 'provider/model' : '裸名'}）</label>
                <input value={d.model} onChange={e => setDraft(a.id, { model: e.target.value })} placeholder="默认" />
              </div>
              <div className="field"><label>快模型（低风险改动，可空 → 回落上方模型）</label>
                <input value={d.fast_model} onChange={e => setDraft(a.id, { fast_model: e.target.value })} placeholder={d.kind === 'opencode' ? 'anthropic/claude-haiku-4-5' : '如 sonnet / haiku'} />
              </div>
              <div className="field"><label>强模型（高风险改动，可空 → 回落上方模型）</label>
                <input value={d.strong_model} onChange={e => setDraft(a.id, { strong_model: e.target.value })} placeholder={d.kind === 'opencode' ? 'anthropic/claude-opus-4-8' : '如 opus'} />
              </div>
              <div className="field"><label>额外参数（空格分隔，可空）</label>
                <input value={d.extra} onChange={e => setDraft(a.id, { extra: e.target.value })} placeholder="--flag value" />
              </div>
              <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 12 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <Switch on={d.enabled} onToggle={() => setDraft(a.id, { enabled: !d.enabled })} />
                  <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-2)' }}>启用</span>
                </div>
                <button className="btn btn-primary btn-sm" onClick={() => save(a)}><Icon name="check" size={13} />保存</button>
                <button className="btn btn-sm" disabled={loadingA} onClick={() => checkAuth(a.id, true)}
                  title="验证 CLI 工具就绪，并用配置的模型发一个极小 prompt 确认模型可用（慢几秒、消耗极少量 token）">
                  <Icon name="shield" size={13} />{loadingA ? '检测中…' : '检测可用性'}</button>
                {!a.is_default && <button className="btn btn-sm btn-danger" style={{ marginLeft: 'auto' }} onClick={() => remove(a)}><Icon name="trash" size={13} />删除</button>}
              </div>
              {probe?.detail && (
                <div className="field full" style={{ marginTop: -4 }}>
                  <span style={{ fontSize: 'var(--text-caption)', color: probe.model === false ? 'var(--red)' : 'var(--text-faint)', fontFamily: 'var(--font-mono)', wordBreak: 'break-word' }}>{probe.detail}</span>
                </div>
              )}
            </div>
            )}
          </div>
        );
      })}

      <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginTop: 4 }}>
        <button className="btn" onClick={addCustom}><Icon name="plus" size={14} />新增自定义 Agent</button>
        {msg && <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{msg}</span>}
      </div>
    </div>
  );
}

interface SkillDraft { name: string; description: string; body: string; project_id: string | null; enabled: boolean; }

function CodeAgentSkillSettings() {
  const [skills, setSkills] = useState<CodeAgentSkillT[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [drafts, setDrafts] = useState<Record<string, SkillDraft>>({});
  const [loading, setLoading] = useState(true);
  const [msg, setMsg] = useState('');
  const [confirmDel, setConfirmDel] = useState<CodeAgentSkillT | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const toggleExpand = (id: string) => setExpanded(prev => {
    const next = new Set(prev);
    next.has(id) ? next.delete(id) : next.add(id);
    return next;
  });

  const toDraft = (s: CodeAgentSkillT): SkillDraft =>
    ({ name: s.name, description: s.description, body: s.body, project_id: s.project_id, enabled: s.enabled });

  const load = () => {
    setLoading(true);
    Promise.all([listCodeAgentSkills(), listProjects().catch(() => [] as Project[])])
      .then(([list, ps]) => {
        setSkills(list);
        setProjects(ps);
        setDrafts(Object.fromEntries(list.map(s => [s.id, toDraft(s)])));
      })
      .catch(() => setSkills([]))
      .finally(() => setLoading(false));
  };
  useEffect(() => { load(); }, []);

  const setDraft = (id: string, patch: Partial<SkillDraft>) =>
    setDrafts(d => ({ ...d, [id]: { ...d[id], ...patch } }));

  const scopeOptions = [
    { value: '', label: '全局（所有项目）' },
    ...projects.map(p => ({ value: p.id, label: p.name })),
  ];

  const save = async (s: CodeAgentSkillT) => {
    const d = drafts[s.id];
    if (!d) return;
    if (!d.name.trim()) { setMsg('技能名不能为空'); return; }
    setMsg('');
    try {
      await upsertCodeAgentSkill({
        id: s.id, name: d.name.trim(), description: d.description.trim(),
        body: d.body, project_id: d.project_id || null, enabled: d.enabled,
      });
      setMsg(`已保存「${d.name.trim()}」`);
      load();
    } catch (e) { setMsg(String(e)); }
  };

  const doDelete = async (s: CodeAgentSkillT) => {
    setConfirmDel(null);
    try { await deleteCodeAgentSkill(s.id); load(); } catch (e) { setMsg(String(e)); }
  };

  const addSkill = async () => {
    setMsg('');
    try {
      const created = await upsertCodeAgentSkill({
        name: 'new-skill', description: '一句话描述：编码 agent 何时该用它', body: '# 技能说明\n在此写明步骤与约束。', enabled: true,
      });
      setExpanded(prev => new Set(prev).add(created.id));
      load();
    } catch (e) { setMsg(String(e)); }
  };

  return (
    <div className="set-inner rise">
      <div className="set-h">编码技能（Skill）</div>
      <div className="set-desc">
        把可复用的「做法/手册」注入到编码 agent 的 worktree，让它在实现需求与解冲突时遵循。
        <b>claude</b> 一等公民——写入 <code>.claude/skills/&lt;name&gt;/SKILL.md</code> 走原生渐进披露
        （名称+描述常驻、正文按需加载）；<b>codex / opencode</b> 无原生 skill 机制，降级为把技能折叠进 prompt。
        注入文件在执行结束即清理、并经 git exclude 兜底，<b>绝不会被提交</b>到代码分支。项目还可在仓内
        手写 <code>.autoforge/skills/&lt;name&gt;/SKILL.md</code>，与这里的全局库取并集（同名时仓内文件优先）。
      </div>

      {loading ? (
        <div className="empty"><Icon name="layers" /><div>加载中…</div></div>
      ) : skills.length === 0 ? (
        <div className="empty"><Icon name="layers" /><div>还没有技能。新增一个，注入编码 agent 的工作区。</div></div>
      ) : skills.map(s => {
        const d = drafts[s.id]; if (!d) return null;
        const isOpen = expanded.has(s.id);
        const scopeName = s.project_id ? (projects.find(p => p.id === s.project_id)?.name ?? '指定项目') : '全局';
        return (
          <div className="panel" key={s.id} style={{ marginBottom: 12 }}>
            <div className="panel-head" style={{ display: 'flex', alignItems: 'center', gap: 10, cursor: 'pointer', userSelect: 'none' }}
              onClick={() => toggleExpand(s.id)}>
              <Icon name="chevron" size={14} style={{ color: 'var(--text-3)', transform: isOpen ? 'rotate(0deg)' : 'rotate(-90deg)', transition: 'transform .15s' }} />
              <span className="chip" style={{ fontFamily: 'var(--font-mono)' }}>{s.name}</span>
              <span style={{ fontWeight: 600, color: 'var(--text-2)', fontSize: 'var(--text-control)' }}>{d.description || '（无描述）'}</span>
              <span className="chip blue" style={{ marginLeft: 'auto' }}>{scopeName}</span>
              {!d.enabled && <span className="chip" style={{ color: 'var(--text-faint)' }}>已停用</span>}
            </div>
            {isOpen && (
              <div className="cfg-fields" style={{ padding: '12px 14px' }}>
                <div className="field"><label>技能名</label>
                  <input value={d.name} onChange={e => setDraft(s.id, { name: e.target.value })} placeholder="race-audit" />
                  <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginTop: 4 }}>清洗为 [A-Za-z0-9_-]，作目录名 / SKILL.md 的 name。</span>
                </div>
                <div className="field"><label>适用范围</label>
                  <Select value={d.project_id ?? ''} onChange={val => setDraft(s.id, { project_id: val || null })} options={scopeOptions} />
                </div>
                <div className="field full"><label>描述（决定 claude 何时按需加载正文，务必精炼准确）</label>
                  <input value={d.description} onChange={e => setDraft(s.id, { description: e.target.value })} placeholder="审查并发竞态：共享状态、await 持锁、TOCTOU、幂等键" />
                </div>
                <div className="field full"><label>正文（SKILL.md，Markdown：写明步骤、清单与约束）</label>
                  <textarea value={d.body} onChange={e => setDraft(s.id, { body: e.target.value })} rows={10}
                    style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-control)', resize: 'vertical' }}
                    placeholder={'# 竞态审查手册\n逐项检查：\n- 共享状态是否有锁\n- 是否在 await 期间持锁\n- 幂等键是否缺失'} />
                </div>
                <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 12 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <Switch on={d.enabled} onToggle={() => setDraft(s.id, { enabled: !d.enabled })} />
                    <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-2)' }}>启用</span>
                  </div>
                  <button className="btn btn-primary btn-sm" onClick={() => save(s)}><Icon name="check" size={13} />保存</button>
                  <button className="btn btn-sm btn-danger" style={{ marginLeft: 'auto' }} onClick={() => setConfirmDel(s)}><Icon name="trash" size={13} />删除</button>
                </div>
              </div>
            )}
          </div>
        );
      })}

      <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginTop: 4 }}>
        <button className="btn" onClick={addSkill}><Icon name="plus" size={14} />新增技能</button>
        {msg && <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>{msg}</span>}
      </div>

      {confirmDel && (
        <ConfirmModal
          msg={`确认删除技能「${confirmDel.name}」？`}
          onOk={() => doDelete(confirmDel)}
          onCancel={() => setConfirmDel(null)}
        />
      )}
    </div>
  );
}

function ConcurrencySettings() {
  const [form, setForm] = useState({ max_slots: 5, pause_threshold: 20, queue_strategy: 'priority', timeout_min: 30, idle_timeout_min: 8, max_load_factor: 1.5, build_slots: 2, cpu_budget_pct: 0 });
  const [result, setResult] = useState('');

  useEffect(() => {
    getConcurrencyConfig().then(cfg => setForm(f => ({
      ...f,
      max_slots: cfg.max_slots, pause_threshold: cfg.pause_threshold, queue_strategy: cfg.queue_strategy,
      timeout_min: cfg.timeout_min, idle_timeout_min: cfg.idle_timeout_min, max_load_factor: cfg.max_load_factor,
      build_slots: cfg.build_slots, cpu_budget_pct: cfg.cpu_budget_pct,
    }))).catch(() => { });
  }, []);

  const save = async () => {
    const cfg = await updateConcurrencyConfig({
      max_slots: form.max_slots,
      pause_threshold: form.pause_threshold,
      queue_strategy: form.queue_strategy,
      timeout_min: form.timeout_min,
      idle_timeout_min: form.idle_timeout_min,
      max_load_factor: form.max_load_factor,
      build_slots: form.build_slots,
      cpu_budget_pct: form.cpu_budget_pct,
    });
    setForm({ max_slots: cfg.max_slots, pause_threshold: cfg.pause_threshold, queue_strategy: cfg.queue_strategy, timeout_min: cfg.timeout_min, idle_timeout_min: cfg.idle_timeout_min, max_load_factor: cfg.max_load_factor, build_slots: cfg.build_slots, cpu_budget_pct: cfg.cpu_budget_pct });
    setResult(`${cfg.stage} · ${cfg.active_slots}/${cfg.max_slots} · 待审核 ${cfg.pending_review}`);
  };

  return (
    <div className="set-inner rise">
      <div className="set-h">并发与流控</div>
      <div className="set-desc">控制代码 Agent 执行槽位、审核积压背压阈值与代码 agent 超时回收。</div>
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
          <div className="field"><label>墙钟超时（分钟）</label>
            <input type="number" min="5" max="180" value={form.timeout_min} onChange={e => setForm(f => ({ ...f, timeout_min: Number(e.target.value) }))} />
            <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginTop: 4 }}>单个代码 agent 运行硬上限，到点杀进程组兜底。</span>
          </div>
          <div className="field"><label>空闲超时（分钟）</label>
            <input type="number" min="0" max="60" value={form.idle_timeout_min} onChange={e => setForm(f => ({ ...f, idle_timeout_min: Number(e.target.value) }))} />
            <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginTop: 4 }}>连续无输出【且无 CPU 消耗】才判卡死并杀进程组；0=关闭。安静的长构建不会误杀。</span>
          </div>
          <div className="field"><label>负载背压（×核数）</label>
            <input type="number" min="0" max="8" step="0.5" value={form.max_load_factor} onChange={e => setForm(f => ({ ...f, max_load_factor: Number(e.target.value) }))} />
            <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginTop: 4 }}>系统负载超过 该值×CPU核数 时暂缓再起新 agent（在已有任务运行时）；0=关闭。压住批量过载。</span>
          </div>
          <div className="field"><label>构建池并发</label>
            <input type="number" min="1" max="16" value={form.build_slots} onChange={e => setForm(f => ({ ...f, build_slots: Number(e.target.value) }))} />
            <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginTop: 4 }}>合并门测试任意时刻最多并发编译数，限住批量合并的编译波。全平台。</span>
          </div>
          <div className="field"><label>CPU 预算（% × 核数）</label>
            <input type="number" min="0" max="100" step="5" value={form.cpu_budget_pct} onChange={e => setForm(f => ({ ...f, cpu_budget_pct: Number(e.target.value) }))} />
            <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginTop: 4 }}>cgroup 把 agent 自测+门的总 CPU 限到 该%×核数（不禁测试、只限速）；0=关闭。仅 Linux 生效，建议 70~80。</span>
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
        <br />本页只管<strong>调度</strong>；Innate 用的<strong>蒸馏 LLM 与 Embedding 模型</strong>在「角色 Agent → 知识层（Innate）」配置。
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

// 单个快捷键绑定面板（启用开关 + 组合键展示 + 录制/恢复默认）。纯前端偏好，落 localStorage
// 并派发 SHORTCUT_CHANGED_EVENT 让 App 实时重读。
function ShortcutBinding({
  storageKey, defaultShortcut, parse, icon, title, enableHint,
}: {
  storageKey: string;
  defaultShortcut: QuickCaptureShortcut;
  parse: (v: string | null | undefined) => QuickCaptureShortcut;
  icon: string;
  title: string;
  enableHint: string;
}) {
  const [shortcut, setShortcut] = useState<QuickCaptureShortcut>(() => parse(localStorage.getItem(storageKey)));
  const [recording, setRecording] = useState(false);
  const persistShortcut = (s: QuickCaptureShortcut) => {
    setShortcut(s);
    localStorage.setItem(storageKey, JSON.stringify(s));
    window.dispatchEvent(new Event(SHORTCUT_CHANGED_EVENT));
  };

  // 录制模式：捕获下一组「修饰键 + 主键」的组合并保存。Esc 取消，单纯修饰键忽略。
  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === 'Escape') { setRecording(false); return; }
      if (isModifierCode(e.code)) return; // 等待主键
      const combo: ShortcutCombo = { ctrl: e.ctrlKey, meta: e.metaKey, alt: e.altKey, shift: e.shiftKey, code: e.code };
      if (!comboHasModifier(combo)) return; // 必须配合至少一个修饰键
      persistShortcut({ enabled: true, combo });
      setRecording(false);
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [recording]);

  return (
    <div className="panel" style={{ marginBottom: 16 }}>
      <div className="panel-head"><div className="panel-title"><Icon name={icon} size={16} style={{ color: 'var(--ember)' }} />{title}</div></div>
      <div style={{ padding: '12px 18px', display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16 }}>
        <div>
          <div style={{ fontSize: 'var(--text-control)', color: 'var(--text)', marginBottom: 3 }}>启用全局快捷键</div>
          <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>{enableHint}</div>
        </div>
        <Switch on={shortcut.enabled} onToggle={() => persistShortcut({ ...shortcut, enabled: !shortcut.enabled })} />
      </div>
      <div style={{ padding: '0 18px 14px', display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
        <span style={{
          fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', letterSpacing: '.06em',
          background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 8,
          padding: '5px 10px', color: shortcut.enabled ? 'var(--ember-soft)' : 'var(--text-3)',
        }}>
          {formatCombo(shortcut.combo)}
        </span>
        <button className="btn btn-sm" onClick={() => setRecording(r => !r)} disabled={!shortcut.enabled}>
          <Icon name="edit" size={13} />{recording ? '按下组合键…' : '录制'}
        </button>
        <button className="btn btn-sm btn-ghost" onClick={() => persistShortcut(defaultShortcut)} disabled={!shortcut.enabled}>
          恢复默认
        </button>
        {recording && (
          <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>
            需配合 Ctrl / Alt / Shift / ⌘ · Esc 取消
          </span>
        )}
      </div>
    </div>
  );
}

function ShortcutSettings() {
  return (
    <div className="set-inner rise">
      <div className="set-h">快捷键</div>
      <div className="set-desc">为常用操作绑定键盘快捷键。组合键需配合至少一个修饰键（Ctrl / Alt / Shift / ⌘），在应用内任意位置生效。</div>

      <ShortcutBinding
        storageKey={QUICK_CAPTURE_SHORTCUT_KEY}
        defaultShortcut={DEFAULT_QUICK_CAPTURE_SHORTCUT}
        parse={parseQuickCaptureShortcut}
        icon="zap"
        title="速录念头"
        enableHint="在应用内任意位置按下快捷键即可弹出「速录念头」，随手把念头丢进待整理池。"
      />

      <ShortcutBinding
        storageKey={VOICE_INPUT_SHORTCUT_KEY}
        defaultShortcut={DEFAULT_VOICE_INPUT_SHORTCUT}
        parse={parseVoiceInputShortcut}
        icon="mic"
        title="快速语音录入"
        enableHint="在任意含语音录入的界面（会议室、速录念头…）一键开/关录音；若当前无语音界面，则弹出「速录念头」并自动起录。"
      />
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

  const [railMode, setRailMode] = useState<RailMode>(() => parseRailMode(localStorage.getItem(RAIL_STORAGE_KEY)));
  useEffect(() => {
    localStorage.setItem(RAIL_STORAGE_KEY, railMode);
    applyRailMode(railMode);
  }, [railMode]);

  // 标题栏系统资源（CPU/内存）监视开关：默认开启，落 localStorage 并广播给 App 实时生效。
  const [resMon, setResMon] = useState<boolean>(() => parseResMonitor(localStorage.getItem(RES_MONITOR_KEY)));
  const toggleResMon = () => {
    setResMon(prev => {
      const next = !prev;
      localStorage.setItem(RES_MONITOR_KEY, next ? 'on' : 'off');
      window.dispatchEvent(new Event(RES_MONITOR_CHANGED_EVENT));
      return next;
    });
  };

  return (
    <div className="set-inner set-inner-wide rise">
      <div className="set-h">主题设置</div>
      <div className="set-desc">当前明暗主题已归入 Forge Ember。选择任一主题族后，可在深色和浅色两种风格间切换。</div>

      <div className="panel" style={{ marginBottom: 16 }}>
        <div className="panel-head"><div className="panel-title"><Icon name="columns" size={16} />导航栏</div></div>
        <div style={{ padding: '12px 18px', display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16 }}>
          <div>
            <div style={{ fontSize: 'var(--text-control)', color: 'var(--text)', marginBottom: 3 }}>悬停展开导航栏</div>
            <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>
              开启后鼠标悬停可展开标签；关闭则锁定为收起状态，悬停不再触发展开。
            </div>
          </div>
          <Switch on={railMode === 'hover'} onToggle={() => setRailMode(m => (m === 'hover' ? 'locked' : 'hover'))} />
        </div>
      </div>

      <div className="panel" style={{ marginBottom: 16 }}>
        <div className="panel-head"><div className="panel-title"><Icon name="cpu" size={16} />标题栏</div></div>
        <div style={{ padding: '12px 18px', display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16 }}>
          <div>
            <div style={{ fontSize: 'var(--text-control)', color: 'var(--text)', marginBottom: 3 }}>显示系统资源占用</div>
            <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>
              在标题栏右侧实时显示当前系统 CPU 与内存占用（每 3 秒刷新）。关闭后不再轮询。
            </div>
          </div>
          <Switch on={resMon} onToggle={toggleResMon} />
        </div>
      </div>

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
          <div>合并入口：只有代码审核批准后可入队 merge</div>
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
              <div className="cfg-sub">{d.issue_id.slice(0, 10)} · {fmtFull(d.created_at)}</div>
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
  const [auth, setAuth] = useState<boolean | null>(null);
  const [authLoading, setAuthLoading] = useState(true);
  const [previews, setPreviews] = useState<PreviewEnvironment[]>([]);
  const [tests, setTests] = useState<TestSession[]>([]);
  const [failures, setFailures] = useState<JobFailure[]>([]);

  const loadHealth = () => {
    setHealthLoading(true);
    setHealthError(false);
    getSystemHealth()
      .then(h => { setHealth(h); setHealthError(false); })
      .catch(() => { setHealth(null); setHealthError(true); })
      .finally(() => setHealthLoading(false));
    // system_health 永远返回 claude_auth=true（避开启动期 SIGTRAP），
    // 真实登录态需走专用的 check_claude_auth 命令（进程组隔离后可安全调用）。
    setAuthLoading(true);
    checkClaudeAuth()
      .then(ok => setAuth(ok))
      .catch(() => setAuth(null))
      .finally(() => setAuthLoading(false));
    listJobFailures(30).then(setFailures).catch(() => setFailures([]));
  };

  useEffect(() => {
    loadHealth();
    listPreviewEnvironments().then(setPreviews).catch(() => setPreviews([]));
    listTestSessions().then(setTests).catch(() => setTests([]));
    listJobFailures(30).then(setFailures).catch(() => setFailures([]));
  }, []);

  const dbVal   = healthLoading ? '…' : health?.db_ok ? 'OK' : healthError ? '错误' : '—';
  const dbColor = healthLoading ? 'var(--text-3)' : health?.db_ok ? 'var(--green)' : healthError ? 'var(--red)' : 'var(--text-3)';
  const authVal   = authLoading ? '…' : auth === null ? '—' : auth ? 'OK' : '未登录';
  const authColor = authLoading ? 'var(--text-3)' : auth === null ? 'var(--text-3)' : auth ? 'var(--green)' : 'var(--red)';
  const stageVal  = healthLoading ? '…'
    : health ? (health.stage === 'paused' ? '已暂停' : health.stage === 'throttled' ? '降速' : '正常')
    : '—';

  return (
    <div className="set-inner rise">
      <div className="set-h">关于 AutoForge</div>
      <div className="set-desc" style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <span>运行健康、Claude 认证和后台运行态概览。</span>
        {healthError && <span style={{ fontSize: 'var(--text-label)', color: 'var(--red)' }}>状态获取失败</span>}
        <button className="btn" style={{ marginLeft: 'auto', fontSize: 'var(--text-label)', padding: '2px 10px' }} onClick={loadHealth} disabled={healthLoading || authLoading}>
          {(healthLoading || authLoading) ? '加载中…' : '刷新'}
        </button>
      </div>
      <div className="stat-grid" style={{ gridTemplateColumns: 'repeat(4, minmax(0, 1fr))', marginBottom: 16 }}>
        {[
          { label: '数据库',      val: dbVal,    color: dbColor,        ic: 'layers' },
          { label: 'Claude Auth', val: authVal,  color: authColor,      ic: 'shield' },
          { label: '版本',        val: health?.version ?? (healthLoading ? '…' : '—'), color: 'var(--blue)', ic: 'package' },
          { label: '阶段',        val: stageVal, color: 'var(--ember)', ic: 'zap' },
        ].map(x => (
          <div className="stat" key={x.label}>
            <div className="stat-ic" style={{ background: `color-mix(in oklab, ${x.color} 16%, transparent)`, color: x.color }}>
              <Icon name={x.ic} size={18} />
            </div>
            <div className="stat-main">
              <div className="stat-label">{x.label}</div>
              <div className="stat-val" style={{ color: x.color, fontSize: 'var(--text-section)' }}>{x.val}</div>
            </div>
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
      <div className="panel" style={{ marginBottom: 16 }}>
        <div className="panel-head"><div className="panel-title"><Icon name="flask" size={16} style={{ color: 'var(--green)' }} />测试会话</div><span className="sec-kicker">{tests.length}</span></div>
        <div style={{ padding: '8px 18px 14px', display: 'grid', gap: 8 }}>
          {tests.slice(0, 8).map(t => <div key={t.id} style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)' }}>{t.status} · {t.summary || t.id}</div>)}
          {tests.length === 0 && <div className="empty-compact" style={{ padding: '0' }}>暂无测试会话</div>}
        </div>
      </div>
      <div className="panel">
        <div className="panel-head"><div className="panel-title"><Icon name="alert" size={16} style={{ color: 'var(--red)' }} />错误历史</div><span className="sec-kicker">{failures.length}</span></div>
        <div style={{ padding: '8px 18px 14px', display: 'grid', gap: 10 }}>
          {failures.slice(0, 20).map(f => (
            <div key={f.id} style={{ display: 'grid', gap: 3, paddingBottom: 8, borderBottom: '1px solid var(--border)' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                <span className="chip red" style={{ fontSize: 'var(--text-micro)' }}>{f.job_type}</span>
                <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>第 {f.attempt} 次 · {fmtFull(f.updated_at)}</span>
              </div>
              {f.last_error && <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)', whiteSpace: 'pre-wrap', wordBreak: 'break-word', lineHeight: 'var(--leading-snug)' }}>{f.last_error.slice(0, 400)}</div>}
            </div>
          ))}
          {failures.length === 0 && <div className="empty-compact" style={{ padding: '0' }}>暂无失败任务</div>}
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

// 各通道类型的展示文案与目标/密钥占位提示。
const NOTIFY_KINDS: { value: string; label: string; targetPh: string; secretLabel?: string; secretPh?: string }[] = [
  { value: 'slack', label: 'Slack', targetPh: 'Slack Incoming Webhook URL' },
  { value: 'wecom', label: '企业微信群机器人', targetPh: '企业微信群机器人 Webhook URL' },
  { value: 'feishu', label: '飞书 (Lark) 机器人', targetPh: '飞书自定义机器人 Webhook URL', secretLabel: '签名密钥', secretPh: '加签 secret（可选，启用签名时填）' },
  { value: 'dingtalk', label: '钉钉机器人', targetPh: '钉钉自定义机器人 Webhook URL', secretLabel: '加签密钥', secretPh: 'SECxxxx 加签密钥（可选）' },
  { value: 'ntfy', label: 'ntfy', targetPh: 'topic URL，如 https://ntfy.sh/your-topic', secretLabel: '访问 Token', secretPh: 'Bearer Token（可选，私有 topic）' },
  { value: 'clawbot', label: '微信ClawBot', targetPh: '扫码绑定后自动填充', secretLabel: 'Bearer Token', secretPh: '扫码绑定后自动填充' },
  { value: 'email', label: 'Email (SMTP)', targetPh: 'smtp://user:pass@host:port?from=a@b&to=c@d' },
  { value: 'webhook', label: '通用 Webhook', targetPh: '通用 Webhook URL' },
];

// 流水线会发出的事件（与后端 dispatch 调用点一致）。空选择 = 订阅全部。
const NOTIFY_EVENTS: { value: string; label: string }[] = [
  { value: 'review_needed', label: '待审核' },
  { value: 'auto_merged', label: '自动合并' },
  { value: 'cr_merged', label: '已合并' },
  { value: 'test_failed', label: '测试失败' },
  { value: 'analysis_failed', label: '分析失败' },
  { value: 'security_high', label: '安全高危' },
];

const emptyNotifyForm = { name: '', kind: 'slack', target: '', secret: '', events: [] as string[], enabled: true };

function NotifySettings() {
  const [channels, setChannels] = useState<NotifyChannel[]>([]);
  const [form, setForm] = useState(emptyNotifyForm);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [busy, setBusy] = useState('');
  const [err, setErr] = useState('');
  // 微信 ClawBot 扫码绑定态
  const [bind, setBind] = useState<{ qrSvg: string; qrUrl: string; status: string; bound: boolean } | null>(null);
  const bindCancel = useRef(false);
  const codeRef = useRef('');
  const [needCode, setNeedCode] = useState(false);

  const reload = () => { listNotifyChannels().then(setChannels).catch(() => setChannels([])); };
  useEffect(() => { reload(); }, []);
  useEffect(() => () => { bindCancel.current = true; }, []);

  const CLAW_STATUS_MSG: Record<string, string> = {
    starting: '正在申请二维码…', wait: '用手机微信扫描二维码以绑定', scaned: '已扫描，请在手机上确认',
    need_verifycode: '请输入手机微信显示的数字验证码后继续', scaned_but_redirect: '正在切换接入节点…',
    expired: '二维码已过期，请重新获取', verify_code_blocked: '验证码多次错误，请稍后重试',
    binded_redirect: '该微信已绑定过此 OpenClaw（无新凭据）', confirmed: '绑定成功',
  };

  const run = async (key: string, fn: () => Promise<unknown>) => {
    setErr(''); setBusy(key);
    try { await fn(); reload(); }
    catch (e) { setErr(String(e)); }
    finally { setBusy(''); }
  };

  const kindMeta = NOTIFY_KINDS.find(k => k.value === form.kind);
  const resetForm = () => { setForm(emptyNotifyForm); setEditingId(null); };

  const toggleEvent = (ev: string) => setForm(f => ({
    ...f, events: f.events.includes(ev) ? f.events.filter(e => e !== ev) : [...f.events, ev],
  }));

  const startEdit = (c: NotifyChannel) => {
    setEditingId(c.id);
    setForm({
      name: c.name, kind: c.kind, target: c.target, secret: '',
      events: c.events.split(',').map(s => s.trim()).filter(Boolean), enabled: c.enabled,
    });
  };

  const save = () => run('save', async () => {
    const payload = {
      name: form.name, kind: form.kind, target: form.target,
      events: form.events.join(','), enabled: form.enabled,
      ...(form.secret.trim() ? { secret: form.secret } : {}),
    };
    if (editingId) await updateNotifyChannel(editingId, payload);
    else await createNotifyChannel(payload);
    resetForm();
  });

  const toggleEnabled = (c: NotifyChannel) => run('en' + c.id, () => updateNotifyChannel(c.id, {
    name: c.name, kind: c.kind, target: c.target, events: c.events, enabled: !c.enabled,
  }));

  const stopBind = () => { bindCancel.current = true; setBind(null); setNeedCode(false); codeRef.current = ''; };

  const startClawbotBind = async () => {
    setErr(''); bindCancel.current = false; codeRef.current = ''; setNeedCode(false);
    setBind({ qrSvg: '', qrUrl: '', status: 'starting', bound: false });
    try {
      const s = await clawbotStartLogin();
      if (bindCancel.current) return;
      setBind({ qrSvg: s.qr_svg, qrUrl: s.qr_url, status: 'wait', bound: false });
      let baseUrl = s.base_url;
      const deadline = Date.now() + 5 * 60_000;
      while (!bindCancel.current && Date.now() < deadline) {
        const r = await clawbotPollLogin(s.qrcode, baseUrl, codeRef.current || undefined);
        if (bindCancel.current) return;
        baseUrl = r.base_url;
        setNeedCode(r.status === 'need_verifycode');
        setBind(b => (b ? { ...b, status: r.status } : b));
        if (r.status === 'confirmed' && r.target && r.bot_token) {
          await createNotifyChannel({
            name: form.name.trim() || '微信ClawBot', kind: 'clawbot', target: r.target,
            secret: r.bot_token, events: form.events.join(','), enabled: form.enabled,
          });
          setBind(b => (b ? { ...b, status: 'confirmed', bound: true } : b));
          resetForm(); reload();
          return;
        }
        if (r.status === 'expired' || r.status === 'verify_code_blocked' || r.status === 'binded_redirect') {
          return; // 终态：保留面板与提示，由用户「重新获取」
        }
        await new Promise(res => setTimeout(res, 1200));
      }
    } catch (e) { setErr(String(e)); setBind(null); }
  };

  const kindLabel = (k: string) => NOTIFY_KINDS.find(x => x.value === k)?.label ?? k;

  return (
    <div className="set-inner rise">
      <div className="set-h">通知通道</div>
      <div className="set-desc">全局通知通道，用于推送流水线事件（审核、部署、安全告警等）。所有项目共享。签名密钥 / Token 加密存储，不回显。</div>
      {err && <div className="chip red" style={{ alignSelf: 'flex-start', marginBottom: 12 }}><Icon name="alert" size={12} />{err}</div>}
      <div className="panel">
        <div className="panel-head">
          <div className="panel-title"><Icon name="bell" size={16} style={{ color: 'var(--ember)' }} />{editingId ? '编辑通道' : '通知通道'}</div>
          <span className="sec-kicker">全局 · {channels.length} 个</span>
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 10, padding: '12px 16px', borderTop: '1px solid var(--border)' }}>
          <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
            <input value={form.name} onChange={e => setForm(f => ({ ...f, name: e.target.value }))} placeholder="名称" style={{ ...notifyInputStyle, width: 140 }} />
            <div style={{ minWidth: 180 }}>
              <Select value={form.kind} onChange={v => { setForm(f => ({ ...f, kind: v })); stopBind(); }}
                options={NOTIFY_KINDS.map(k => ({ value: k.value, label: k.label }))} />
            </div>
            {form.kind !== 'clawbot' && (
              <input value={form.target} onChange={e => setForm(f => ({ ...f, target: e.target.value }))} placeholder={kindMeta?.targetPh ?? 'URL'} style={{ ...notifyInputStyle, flex: 1, minWidth: 220 }} />
            )}
            {form.kind === 'clawbot' && editingId && (
              <span style={{ flex: 1, minWidth: 220, fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-faint)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{form.target || '（已绑定）'}</span>
            )}
          </div>
          {kindMeta?.secretLabel && form.kind !== 'clawbot' && (
            <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '.06em', width: 80 }}>{kindMeta.secretLabel}</span>
              <input type="password" autoComplete="new-password" value={form.secret} onChange={e => setForm(f => ({ ...f, secret: e.target.value }))}
                placeholder={editingId ? (kindMeta.secretPh + '（留空保留原值）') : kindMeta.secretPh} style={{ ...notifyInputStyle, flex: 1 }} />
            </div>
          )}
          {form.kind === 'clawbot' && !editingId && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 10, padding: '12px', borderRadius: 9, background: 'var(--bg-3)', border: '1px solid var(--border)' }}>
              {!bind && (
                <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
                  <button className="btn btn-primary btn-sm" disabled={!form.name.trim()} onClick={startClawbotBind}>
                    <Icon name="bot" size={14} />扫码绑定
                  </button>
                  <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>先填名称，点击后用手机微信扫码绑定 ClawBot</span>
                </div>
              )}
              {bind && (
                <div style={{ display: 'flex', gap: 14, alignItems: 'flex-start', flexWrap: 'wrap' }}>
                  <div style={{ width: 160, height: 160, borderRadius: 9, background: '#fff', display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
                    {bind.qrSvg
                      ? <img src={bind.qrSvg} alt="ClawBot 绑定二维码" style={{ width: 152, height: 152 }} />
                      : <span className="typing" style={{ color: 'var(--bg)' }} />}
                  </div>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 8, flex: 1, minWidth: 200 }}>
                    <div style={{ fontSize: 'var(--text-control)', color: bind.bound ? 'var(--green)' : 'var(--text-2)', display: 'flex', alignItems: 'center', gap: 6 }}>
                      {bind.bound && <Icon name="check" size={14} />}{CLAW_STATUS_MSG[bind.status] ?? bind.status}
                    </div>
                    {needCode && !bind.bound && (
                      <input autoFocus placeholder="输入手机上显示的数字" onChange={e => { codeRef.current = e.target.value.trim(); }}
                        style={{ ...notifyInputStyle, width: 200 }} />
                    )}
                    {bind.qrUrl && !bind.bound && (
                      <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', wordBreak: 'break-all' }}>二维码无法显示？也可在手机打开：{bind.qrUrl}</span>
                    )}
                    <div style={{ display: 'flex', gap: 6 }}>
                      {(bind.status === 'expired' || bind.status === 'verify_code_blocked' || bind.status === 'binded_redirect') && !bind.bound && (
                        <button className="btn btn-sm" onClick={startClawbotBind}><Icon name="refresh" size={13} />重新获取</button>
                      )}
                      {!bind.bound && <button className="btn btn-sm btn-ghost" onClick={stopBind}>取消</button>}
                    </div>
                  </div>
                </div>
              )}
            </div>
          )}
          <div style={{ display: 'flex', gap: 6, alignItems: 'center', flexWrap: 'wrap' }}>
            <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '.06em', marginRight: 4 }}>订阅事件</span>
            {NOTIFY_EVENTS.map(ev => (
              <button key={ev.value} type="button" className={`filter-chip${form.events.includes(ev.value) ? ' on' : ''}`}
                onClick={() => toggleEvent(ev.value)}>{ev.label}</button>
            ))}
            <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>{form.events.length === 0 ? '（全部）' : ''}</span>
          </div>
          <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            {!(form.kind === 'clawbot' && !editingId) && (
              <button className="btn btn-primary btn-sm" disabled={!form.name.trim() || !form.target.trim() || busy === 'save'}
                onClick={save}>
                <Icon name={editingId ? 'check' : 'plus'} size={14} />{editingId ? '保存' : '添加'}
              </button>
            )}
            {editingId && <button className="btn btn-sm btn-ghost" onClick={resetForm}>取消</button>}
          </div>
        </div>
        {channels.map(c => (
          <div key={c.id} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, padding: '10px 16px', borderTop: '1px solid var(--border)', opacity: c.enabled ? 1 : 0.55 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
              <button type="button" className={`switch${c.enabled ? ' on' : ''}`} aria-checked={c.enabled} role="switch"
                onClick={() => toggleEnabled(c)} disabled={busy === 'en' + c.id}><i /></button>
              <span className="chip">{kindLabel(c.kind)}</span>
              <span style={{ fontWeight: 600 }}>{c.name}</span>
              {c.has_secret && <Icon name="lock" size={12} style={{ color: 'var(--text-faint)' }} />}
              <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-faint)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', maxWidth: 220 }}>{c.target}</span>
              {c.events.trim() && <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>· {c.events.split(',').filter(Boolean).length} 事件</span>}
            </div>
            <div style={{ display: 'flex', gap: 6 }}>
              <button className="btn btn-sm" disabled={busy === 'test' + c.id} onClick={() => run('test' + c.id, () => testNotifyChannel(c.id))}><Icon name="send" size={13} />测试</button>
              <button className="btn btn-sm" onClick={() => startEdit(c)}><Icon name="edit" size={13} /></button>
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
  const [autoConflictOn, setAutoConflictOn] = useState(false);
  const [customMsgOn, setCustomMsgOn] = useState(false);
  const [parallelOn, setParallelOn] = useState(false);
  // 各面板说明默认收起，点标题区展开，省高度。键：autopass/conflict/custommsg/parallel。
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const toggleExpand = (k: string) => setExpanded(e => ({ ...e, [k]: !e[k] }));
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState('');

  const reload = () => {
    listAutoPassPolicy().then(setPolicies).catch(() => setPolicies([]));
    getAutoPassEnabled().then(setAutoPassOn).catch(() => {});
    getAutoConflictResolveEnabled().then(setAutoConflictOn).catch(() => {});
    getCustomMergeMessageEnabled().then(setCustomMsgOn).catch(() => {});
    getParallelPremergeEnabled().then(setParallelOn).catch(() => {});
  };
  useEffect(() => { reload(); }, []);

  const toggle = async () => {
    setErr(''); setBusy(true);
    try { const next = !autoPassOn; await setAutoPassEnabled(next); setAutoPassOn(next); }
    catch (e) { setErr(String(e)); }
    finally { setBusy(false); }
  };

  const toggleConflict = async () => {
    setErr(''); setBusy(true);
    try { const next = !autoConflictOn; await setAutoConflictResolveEnabled(next); setAutoConflictOn(next); }
    catch (e) { setErr(String(e)); }
    finally { setBusy(false); }
  };

  const toggleCustomMsg = async () => {
    setErr(''); setBusy(true);
    try { const next = !customMsgOn; await setCustomMergeMessageEnabled(next); setCustomMsgOn(next); }
    catch (e) { setErr(String(e)); }
    finally { setBusy(false); }
  };

  const toggleParallel = async () => {
    setErr(''); setBusy(true);
    try { const next = !parallelOn; await setParallelPremergeEnabled(next); setParallelOn(next); }
    catch (e) { setErr(String(e)); }
    finally { setBusy(false); }
  };

  return (
    <div className="set-inner rise">
      <div className="set-h">合并与放行</div>
      <div className="set-desc">合并环节的策略与自动化：自动放行（门控降级）、冲突自动解决、自定义合并提交信息、合并并行化。</div>
      {err && <div className="chip red" style={{ alignSelf: 'flex-start', marginBottom: 12 }}><Icon name="alert" size={12} />{err}</div>}
      <div className="panel">
        <div className="panel-head">
          <div className="panel-title" onClick={() => toggleExpand('autopass')} style={{ cursor: 'pointer' }}>
            <Icon name="chevron" size={14} style={{ color: 'var(--text-3)', transition: 'transform .15s', transform: expanded.autopass ? 'none' : 'rotate(-90deg)' }} />
            <Icon name="sliders" size={16} style={{ color: 'var(--ember)' }} />门控降级（自动放行）
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span className={'chip ' + (autoPassOn ? 'green' : '')}>{autoPassOn ? '已启用' : '已关闭'}</span>
            <button className="btn btn-sm" disabled={busy} onClick={toggle}>
              <Icon name={autoPassOn ? 'pause' : 'play'} size={13} />{autoPassOn ? '关闭' : '启用'}
            </button>
          </div>
        </div>
        {expanded.autopass && (<>
          <div style={{ padding: '10px 16px', fontSize: 'var(--text-control)', color: 'var(--text-3)', borderTop: '1px solid var(--border)' }}>
            启用后，低风险(T0/T1)且变更类信任达标（连续 20 次批准、0 退改）的改动将自动跳过代码审核直接合并；T3 硬地板（迁移/auth/支付/依赖）永远人工。任一退改清零重挣。
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
        </>)}
      </div>
      <div className="panel" style={{ marginTop: 14 }}>
        <div className="panel-head">
          <div className="panel-title" onClick={() => toggleExpand('conflict')} style={{ cursor: 'pointer' }}>
            <Icon name="chevron" size={14} style={{ color: 'var(--text-3)', transition: 'transform .15s', transform: expanded.conflict ? 'none' : 'rotate(-90deg)' }} />
            <Icon name="zap" size={16} style={{ color: 'var(--ember)' }} />冲突自动解决（AI）
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span className={'chip ' + (autoConflictOn ? 'green' : '')}>{autoConflictOn ? '已启用' : '已关闭'}</span>
            <button className="btn btn-sm" disabled={busy} onClick={toggleConflict}>
              <Icon name={autoConflictOn ? 'pause' : 'play'} size={13} />{autoConflictOn ? '关闭' : '启用'}
            </button>
          </div>
        </div>
        {expanded.conflict && (
        <div style={{ padding: '10px 16px', fontSize: 'var(--text-control)', color: 'var(--text-3)', borderTop: '1px solid var(--border)' }}>
          启用后，合并前自动把 dev 并入分支若发生代码冲突，将直接交由 AI 自动解冲突；解完仍会回到代码审核复审，不直接落 dev。关闭时冲突停在「合并冲突」态，可在审核页手动重试或点 AI 解冲突。
        </div>
        )}
      </div>
      <div className="panel" style={{ marginTop: 14 }}>
        <div className="panel-head">
          <div className="panel-title" onClick={() => toggleExpand('custommsg')} style={{ cursor: 'pointer' }}>
            <Icon name="chevron" size={14} style={{ color: 'var(--text-3)', transition: 'transform .15s', transform: expanded.custommsg ? 'none' : 'rotate(-90deg)' }} />
            <Icon name="merge" size={16} style={{ color: 'var(--ember)' }} />自定义合并提交信息
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span className={'chip ' + (customMsgOn ? 'green' : '')}>{customMsgOn ? '已启用' : '已关闭'}</span>
            <button className="btn btn-sm" disabled={busy} onClick={toggleCustomMsg}>
              <Icon name={customMsgOn ? 'pause' : 'play'} size={13} />{customMsgOn ? '关闭' : '启用'}
            </button>
          </div>
        </div>
        {expanded.custommsg && (
        <div style={{ padding: '10px 16px', fontSize: 'var(--text-control)', color: 'var(--text-3)', borderTop: '1px solid var(--border)' }}>
          启用后，代码审核页「批准合并」前可编辑本次合并的提交信息（merge --no-ff -m）。关闭时合并统一使用默认模板 <code>&lt;前缀&gt;(&lt;修改模块&gt;): &lt;需求标题&gt; [autoforge #&lt;编号&gt;]</code>（前缀按需求类别取 feat/fix/refactor/chore…），审核页不显示输入框。批量审核始终走默认模板。
        </div>
        )}
      </div>
      <div className="panel" style={{ marginTop: 14 }}>
        <div className="panel-head">
          <div className="panel-title" onClick={() => toggleExpand('parallel')} style={{ cursor: 'pointer' }}>
            <Icon name="chevron" size={14} style={{ color: 'var(--text-3)', transition: 'transform .15s', transform: expanded.parallel ? 'none' : 'rotate(-90deg)' }} />
            <Icon name="zap" size={16} style={{ color: 'var(--ember)' }} />合并并行化（实验）
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span className={'chip ' + (parallelOn ? 'green' : '')}>{parallelOn ? '已启用' : '已关闭'}</span>
            <button className="btn btn-sm" disabled={busy} onClick={toggleParallel}>
              <Icon name={parallelOn ? 'pause' : 'play'} size={13} />{parallelOn ? '关闭' : '启用'}
            </button>
          </div>
        </div>
        {expanded.parallel && (
        <div style={{ padding: '10px 16px', fontSize: 'var(--text-control)', color: 'var(--text-3)', borderTop: '1px solid var(--border)' }}>
          启用后，合并拆为「合并前测试（premerge）」与「落地（land）」两段：测试不再占用项目合并锁，同项目多个待合并需求可并行测试（受构建池约束），仅廉价的落地串行执行——批量合并显著提速。落地前会再校验 dev 是否被其它需求推进：未动或改动互不相交则直接落地，相交则自动回退重测。关闭时走原单锁流程，行为不变。
        </div>
        )}
      </div>
    </div>
  );
}

function WebhookSettings() {
  const [cfg, setCfg]     = useState<IntakeConfig | null>(null);
  const [status, setStatus] = useState<WebhookStatus | null>(null);
  const [form, setForm]   = useState({ enabled: false, port: '27182' });
  const [saving, setSaving]   = useState(false);
  const [copied, setCopied]   = useState(false);
  const [saveOk, setSaveOk]   = useState<boolean | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    Promise.all([getIntakeConfig(), getWebhookStatus()])
      .then(([c, s]) => {
        setCfg(c);
        setStatus(s);
        setForm({ enabled: c.webhook_enabled, port: String(c.webhook_port) });
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
      });
      setCfg(updated);
      setSaveOk(true);
      getWebhookStatus().then(setStatus).catch(() => {});
      setTimeout(() => setSaveOk(null), 2500);
    } catch { setSaveOk(false); }
    finally { setSaving(false); }
  };

  // token 不再全局设置：项目级 webhook token 在「项目管理 → 需求入口 → Webhook」签发。
  const curlExample = `curl -X POST http://127.0.0.1:${form.port}/webhook/issues \\
  -H "Authorization: Bearer <项目 webhook token>" \\
  -H "Content-Type: application/json" \\
  -d '{"title":"需求标题","description":"详细描述"}'`;

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
        </div>

        <div style={{ background: 'rgba(139,122,216,.08)', border: '1px solid rgba(139,122,216,.22)', borderRadius: 10, padding: '10px 14px', fontSize: 'var(--text-label)', color: 'var(--text-2)', display: 'flex', gap: 8, marginBottom: 14 }}>
          <Icon name="bell" size={13} style={{ flexShrink: 0, marginTop: 1, color: 'var(--violet)' }} />
          <div>Webhook 仅监听 <code style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)' }}>127.0.0.1</code>（本机），不暴露公网。接入凭证已改为<strong>项目级 token</strong>——每个项目在「项目管理 → 需求入口 → Webhook」各自签发、可独立吊销；请求落到哪个项目由 token 决定。更改端口/开关后点击「保存并应用」以重启服务。</div>
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

// ── 自更新（AutoForge 管理自身仓库时的安全同步）──────────────────────────────
function SelfUpdateSettings() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [pid, setPid] = useState('');
  const [st, setSt] = useState<SelfUpdateStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [pulling, setPulling] = useState(false);
  const [result, setResult] = useState<SelfUpdateResult | null>(null);

  useEffect(() => {
    listProjects().then(ps => {
      setProjects(ps);
      if (ps.length) setPid(ps[0].id);
    }).catch(() => {});
  }, []);

  // keepResult: after a pull we re-read status but must NOT wipe the just-set
  // pull message (the old code cleared it here → failures silently vanished).
  const refresh = async (id: string, keepResult = false) => {
    if (!id) return;
    setLoading(true); if (!keepResult) setResult(null); setSt(null);
    try { setSt(await selfUpdateStatus(id)); }
    catch (e) { if (!keepResult) setResult({ ok: false, pulled: 0, message: String(e), restart_required: false }); }
    finally { setLoading(false); }
  };
  useEffect(() => { refresh(pid); /* eslint-disable-next-line */ }, [pid]);

  const doPull = async () => {
    setConfirm(false); setPulling(true); setResult(null);
    try { setResult(await selfUpdatePull(pid)); }
    catch (e) { setResult({ ok: false, pulled: 0, message: String(e), restart_required: false }); }
    finally { setPulling(false); refresh(pid, true); }
  };

  return (
    <div className="set-inner rise">
      <div className="set-h">同步更新（自更新）</div>
      <div className="set-desc">
        当 AutoForge 用自身作为项目运行时，交付合并会在隔离 worktree 中完成并<strong>推送到 origin/dev</strong>，不改动正在运行的工作区。
        在此一键拉取最新代码（优先快进；当本地有自己的提交、与远端分叉时自动改用 rebase 把本地提交重放到最新之上，未提交改动会自动暂存/恢复，冲突则回滚不丢失）。
        <br /><strong>注意</strong>：拉取会改动源码，开发模式将<strong>自动重新编译并重启</strong>，请先确认无未保存/未提交的工作。
      </div>
      <div className="cfg-card">
        <div className="cfg-fields">
          <div className="field full"><label>项目</label>
            <Select value={pid} onChange={setPid}
              options={projects.map(p => ({ value: p.id, label: p.name }))} placeholder="选择项目" />
          </div>

          {loading && <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>加载状态中…</div>}

          {st && (
            <div className="field full" style={{ gap: 8 }}>
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, alignItems: 'center' }}>
                <span className="chip">{st.branch || '游离 HEAD'}</span>
                {st.is_self_managed
                  ? <span className="chip ember">自管理仓库</span>
                  : <span className="chip">普通项目</span>}
                {st.behind > 0
                  ? <span className="chip amber">落后 {st.behind}</span>
                  : <span className="chip green">已最新</span>}
                {st.ahead > 0 && <span className="chip blue">领先 {st.ahead}</span>}
                {st.dirty && <span className="chip red">有未提交改动</span>}
              </div>
              <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)' }}>{st.repo_path}</span>
            </div>
          )}

          <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
            <button className="btn btn-primary" disabled={pulling || loading || !st || st.behind === 0}
              onClick={() => setConfirm(true)}>
              <Icon name="refresh" size={14} />{pulling ? '同步中…' : '同步更新'}
              {st && st.behind > 0 && (
                <span className="set-nav-badge" style={{ marginLeft: 6 }}>{st.behind}</span>
              )}
            </button>
          </div>

          {result && (
            <div className="field full" style={{
              flexDirection: 'row', alignItems: 'flex-start', gap: 10, padding: '12px 14px', borderRadius: 'var(--radius-sm)',
              background: result.ok
                ? 'color-mix(in srgb, var(--green) 12%, transparent)'
                : 'color-mix(in srgb, var(--red) 12%, transparent)',
              border: `1px solid color-mix(in srgb, ${result.ok ? 'var(--green)' : 'var(--red)'} 45%, transparent)`,
            }}>
              <Icon name={result.ok ? 'check' : 'alert'} size={16}
                style={{ flexShrink: 0, marginTop: 1, color: result.ok ? 'var(--green-soft)' : 'var(--red)' }} />
              <span style={{ fontSize: 'var(--text-label)', fontFamily: 'var(--font-mono)', whiteSpace: 'pre-wrap',
                lineHeight: 'var(--leading-normal)', color: result.ok ? 'var(--green-soft)' : 'var(--red)' }}>
                {result.message}
              </span>
            </div>
          )}
        </div>
      </div>

      {confirm && createPortal(
        <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', display: 'grid', placeItems: 'center', zIndex: 9999 }}>
          <div style={{ background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 14, padding: '22px 24px', width: 400, boxShadow: 'var(--shadow-lg)' }} onClick={e => e.stopPropagation()}>
            <p style={{ margin: '0 0 14px', fontSize: 'var(--text-body)', lineHeight: 'var(--leading-relaxed)' }}>
              将拉取 origin/{st?.branch || 'dev'} 的 {st?.behind ?? 0} 个新提交到本地。
            </p>
            <p style={{ margin: '0 0 20px', fontSize: 'var(--text-label)', lineHeight: 'var(--leading-relaxed)', color: 'var(--amber-soft)' }}>
              ⚠ 源码将被更新，开发模式会<strong>自动重新编译并重启</strong>（运行中状态丢失）。
              {st?.dirty && ' 当前有未提交改动；若与更新冲突，git 会拒绝拉取以免覆盖你的工作。'}
              请确认已保存/提交重要工作后再继续。
            </p>
            <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
              <button className="btn" onClick={() => setConfirm(false)}>取消</button>
              <button className="btn btn-primary" onClick={doPull}><Icon name="refresh" size={14} />确认同步并接受重启</button>
            </div>
          </div>
        </div>,
        document.body,
      )}
    </div>
  );
}

// ── 配置备份：口令加密导出 / 一键导入 ─────────────────────────────────────────
function BackupSettings() {
  const [secretBackend, setSecretBackend] = useState<SecretBackend | null>(null);
  const [exportPass, setExportPass] = useState('');
  const [exportPass2, setExportPass2] = useState('');
  const [exporting, setExporting] = useState(false);
  const [exportResult, setExportResult] = useState<ExportResult | null>(null);
  const [exportErr, setExportErr] = useState('');

  const [importPass, setImportPass] = useState('');
  const [importing, setImporting] = useState(false);
  const [importResult, setImportResult] = useState<BackupSummary | null>(null);
  const [importErr, setImportErr] = useState('');
  const [pickedName, setPickedName] = useState('');
  const [pickedWarn, setPickedWarn] = useState('');
  const fileRef = useRef<HTMLInputElement>(null);
  const pickedContent = useRef<string>('');

  useEffect(() => { getSecretBackendStatus().then(setSecretBackend).catch(() => {}); }, []);

  const summaryLine = (s: BackupSummary) =>
    `${s.llm_configs} 个 LLM · ${s.agents} 个 Agent · ${s.mcp_servers} 个 MCP · ${s.notify_channels} 个通知 · ${s.app_settings} 项系统设置`;

  const doExport = async () => {
    setExportErr(''); setExportResult(null);
    if (exportPass.length < 6) { setExportErr('口令至少 6 位'); return; }
    if (exportPass !== exportPass2) { setExportErr('两次输入的口令不一致'); return; }
    setExporting(true);
    try {
      const r = await exportConfig(exportPass);
      setExportResult(r);
      setExportPass(''); setExportPass2('');
    } catch (e) {
      setExportErr(String(e));
    } finally { setExporting(false); }
  };

  const onPick = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const f = e.target.files?.[0];
    e.target.value = ''; // 允许再次选择同一文件
    if (!f) return;
    setImportErr(''); setImportResult(null); setPickedWarn('');
    try {
      pickedContent.current = await f.text();
      setPickedName(f.name);
      // 后缀不符仅提示、不阻断：读取不依赖扩展名，但提醒用户可能选错文件
      if (!f.name.toLowerCase().endsWith('.afbackup')) {
        setPickedWarn('该文件后缀不是 .afbackup，可能不是备份文件——请确认无误后再导入。');
      }
    } catch {
      setImportErr('读取文件失败');
      setPickedName('');
      pickedContent.current = '';
    }
  };

  const doImport = async () => {
    setImportErr(''); setImportResult(null);
    if (!pickedContent.current) { setImportErr('请先选择备份文件'); return; }
    if (importPass.length < 6) { setImportErr('请输入导出时设置的口令'); return; }
    setImporting(true);
    try {
      const s = await importConfig(pickedContent.current, importPass);
      setImportResult(s);
      setImportPass('');
    } catch (e) {
      setImportErr(String(e));
    } finally { setImporting(false); }
  };

  return (
    <div className="set-inner rise">
      <div className="set-h">配置备份</div>
      <div className="set-desc">
        把 LLM 配置、角色 Agent、工具 / MCP、通知通道与系统设置（含 API Key 等密钥）整包口令加密导出。
        换机或重装后用同一口令一键导入，快速进入生产。
      </div>
      {secretBackend && (
        <div className="chip" style={{ marginBottom: 14 }}
          title="导出会用本机主密钥还原密钥明文，再以你设置的口令重新加密；导出文件不含本机主密钥，仅口令可解。">
          <Icon name={secretBackend === 'keychain' ? 'shield' : 'alert'} size={11} style={{ verticalAlign: -1, marginRight: 4 }} />
          密钥随包加密 · 仅凭口令可解
        </div>
      )}

      {/* 导出 */}
      <div className="cfg-card" style={{ padding: 16 }}>
        <div className="cfg-name" style={{ marginBottom: 4 }}><Icon name="download" size={15} style={{ verticalAlign: -2, marginRight: 6 }} />加密导出</div>
        <div className="cfg-sub" style={{ marginBottom: 12 }}>设置一个加密口令（至少 6 位）。请牢记口令——丢失则无法解开备份。</div>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
          <div className="field"><label>加密口令</label>
            <input type="password" autoComplete="new-password" value={exportPass}
              onChange={e => setExportPass(e.target.value)} placeholder="至少 6 位" /></div>
          <div className="field"><label>确认口令</label>
            <input type="password" autoComplete="new-password" value={exportPass2}
              onChange={e => setExportPass2(e.target.value)} placeholder="再次输入" /></div>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginTop: 12 }}>
          <button className="btn btn-primary" disabled={exporting} onClick={doExport}>
            <Icon name="download" size={14} />{exporting ? '导出中…' : '加密导出'}
          </button>
          {exportErr && <span style={{ color: 'var(--red)', fontSize: 'var(--text-label)' }}>{exportErr}</span>}
        </div>
        {exportResult && (
          <div className="chip green" style={{ display: 'block', marginTop: 12, padding: '10px 12px', lineHeight: 1.6, borderRadius: 10 }}>
            <div>已导出：{summaryLine(exportResult.summary)}</div>
            <div style={{ color: 'var(--text-3)', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', wordBreak: 'break-all', marginTop: 4 }}>{exportResult.path}</div>
            <button className="btn btn-sm" style={{ marginTop: 8 }} onClick={() => revealBackup(exportResult.path).catch(() => {})}>
              <Icon name="folderOpen" size={13} />在文件夹中显示
            </button>
          </div>
        )}
      </div>

      {/* 导入 */}
      <div className="cfg-card" style={{ padding: 16, marginTop: 14 }}>
        <div className="cfg-name" style={{ marginBottom: 4 }}><Icon name="upload" size={15} style={{ verticalAlign: -2, marginRight: 6 }} />导入恢复</div>
        <div className="cfg-sub" style={{ marginBottom: 12 }}>选择 .afbackup 备份文件并输入当时的口令。按主键合并恢复，不会删除现有未涉及的配置。</div>
        {/* 不设 accept 过滤：Linux WebKitGTK 按 MIME 匹配，会把未注册 MIME 的 .afbackup 文件过滤掉，导致选不到备份文件 */}
        <input ref={fileRef} type="file" style={{ display: 'none' }} onChange={onPick} />
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
          <button className="btn" onClick={() => fileRef.current?.click()}>
            <Icon name="folderOpen" size={14} />选择备份文件
          </button>
          {pickedName && <span className="chip" style={{ fontFamily: 'var(--font-mono)' }}>{pickedName}</span>}
        </div>
        {pickedWarn && (
          <div className="chip amber" style={{ marginTop: 10 }}>
            <Icon name="alert" size={11} style={{ verticalAlign: -1, marginRight: 4 }} />{pickedWarn}
          </div>
        )}
        <div className="field" style={{ marginTop: 12, maxWidth: 320 }}><label>解密口令</label>
          <input type="password" autoComplete="off" value={importPass}
            onChange={e => setImportPass(e.target.value)} placeholder="导出时设置的口令" /></div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginTop: 12 }}>
          <button className="btn" disabled={importing || !pickedName} onClick={doImport}>
            <Icon name="upload" size={14} />{importing ? '导入中…' : '导入并恢复'}
          </button>
          {importErr && <span style={{ color: 'var(--red)', fontSize: 'var(--text-label)' }}>{importErr}</span>}
        </div>
        {importResult && (
          <div className="chip green" style={{ display: 'block', marginTop: 12, padding: '10px 12px', lineHeight: 1.6 }}>
            已恢复：{summaryLine(importResult)}。部分设置（并发等）将在下次启动后生效。
          </div>
        )}
      </div>
    </div>
  );
}

const SET_GROUPS: { group: string; items: { id: string; name: string; ic: string }[] }[] = [
  {
    group: 'AI 核心', // LLM 底座 → 角色/代码 Agent → 能力扩展 → 记忆层
    items: [
      { id: 'llm',         name: 'LLM 配置',     ic: 'brain' },
      { id: 'roles',       name: '角色 Agent',   ic: 'bot' },
      { id: 'codeagent',   name: '代码 Agent',   ic: 'code' },
      { id: 'codeskills',  name: '编码技能',     ic: 'layers' },
      { id: 'tools',       name: '工具 & MCP',   ic: 'search' },
      { id: 'knowledge',   name: '知识库自成长', ic: 'brain' },
    ],
  },
  {
    group: '运行与流控', // 供料入口 → 并发控制 → 合并放行 → 安全守卫
    items: [
      { id: 'autosupply',  name: '自动供料',     ic: 'refresh' },
      { id: 'concurrency', name: '并发与流控',   ic: 'cpu' },
      { id: 'gating',      name: '合并与放行',   ic: 'sliders' },
      { id: 'security',    name: '安全与权限',   ic: 'shield' },
    ],
  },
  {
    group: '集成与通知', // 输入：Webhook / 语音 → 输出：通知
    items: [
      { id: 'webhook',     name: 'Webhook 集成', ic: 'zap' },
      { id: 'asr',         name: '语音录入',     ic: 'mic' },
      { id: 'notify',      name: '通知通道',     ic: 'bell' },
    ],
  },
  {
    group: '系统', // 运维：更新/备份 → 偏好：外观/快捷键 → 关于
    items: [
      { id: 'selfupdate',  name: '同步更新',     ic: 'refresh' },
      { id: 'backup',      name: '配置备份',     ic: 'download' },
      { id: 'theme',       name: '主题设置',     ic: 'palette' },
      { id: 'shortcuts',   name: '快捷键',       ic: 'zap' },
      { id: 'about',       name: '关于 AutoForge', ic: 'box' },
    ],
  },
];
const SET_ITEMS = SET_GROUPS.flatMap(g => g.items);

export default function SettingsPage({
  theme,
  onThemeChange,
}: {
  theme: ThemeSelection;
  onThemeChange: React.Dispatch<React.SetStateAction<ThemeSelection>>;
}) {
  const [sec, setSec] = useState('llm');
  const cur = SET_ITEMS.find(i => i.id === sec)!;

  // 分组折叠：默认只展开当前激活项所在组，其余收起，避免一次铺开全部。
  const [collapsed, setCollapsed] = useState<Set<string>>(() => {
    const open = SET_GROUPS.find(g => g.items.some(it => it.id === sec))?.group;
    return new Set(SET_GROUPS.map(g => g.group).filter(name => name !== open));
  });
  const toggleGroup = (name: string) =>
    setCollapsed(prev => {
      const next = new Set(prev);
      next.has(name) ? next.delete(name) : next.add(name);
      return next;
    });

  // 待拉取提交数角标：每分钟检查一次自管理仓库落后 origin/dev 的提交数。
  const [pendingBehind, setPendingBehind] = useState(0);
  useEffect(() => {
    let alive = true;
    const tick = () => selfUpdatePending()
      .then(r => { if (alive) setPendingBehind(r.behind); })
      .catch(() => {});
    tick();
    const t = setInterval(tick, 60_000);
    return () => { alive = false; clearInterval(t); };
  }, []);
  return (
    <div className="content">
      <div className="audit-top" style={{ height: 56 }}>
        <div className="eyebrow" style={{ fontSize: 'var(--text-heading)' }}><span className="en">SETTINGS</span><span className="cn">· 设置</span></div>
      </div>
      <div className="set-wrap">
        <div className="set-nav">
          {SET_GROUPS.map(g => {
            const isCollapsed = collapsed.has(g.group);
            // 折叠态仍有该组项被选中时，用角标提示当前位置不丢失。
            const hasActive = g.items.some(it => it.id === sec);
            return (
              <Fragment key={g.group}>
                <div className={'set-nav-group' + (isCollapsed ? ' collapsed' : '')} onClick={() => toggleGroup(g.group)}>
                  {g.group}
                  {isCollapsed && hasActive && <span className="dot ember" style={{ marginLeft: 4 }} />}
                  <Icon name="chevron" size={14} className="set-nav-chevron" />
                </div>
                {!isCollapsed && g.items.map(it => (
                  <div key={it.id} className={'set-nav-item' + (sec === it.id ? ' active' : '')} onClick={() => setSec(it.id)}>
                    <Icon name={it.ic} size={18} />{it.name}
                    {it.id === 'selfupdate' && pendingBehind > 0 && (
                      <span className="set-nav-badge" title={`有 ${pendingBehind} 个提交待拉取`}>{pendingBehind}</span>
                    )}
                  </div>
                ))}
              </Fragment>
            );
          })}
        </div>
        <div className="set-body scroll">
          {sec === 'theme'       && <ThemeSettings theme={theme} onThemeChange={onThemeChange} />}
          {sec === 'shortcuts'   && <ShortcutSettings />}
          {sec === 'llm'         && <LLMSettings />}
          {sec === 'tools'       && <ToolsSettings />}
          {sec === 'roles'       && <RolesPage />}
          {sec === 'concurrency' && <ConcurrencySettings />}
          {sec === 'codeagent'   && <CodeAgentSettings />}
          {sec === 'codeskills'  && <CodeAgentSkillSettings />}
          {sec === 'selfupdate'  && <SelfUpdateSettings />}
          {sec === 'backup'      && <BackupSettings />}
          {sec === 'knowledge'   && <KnowledgeSettings />}
          {sec === 'security'    && <SecuritySettings />}
          {sec === 'asr'         && <AsrSettings />}
          {sec === 'autosupply'  && <AutosupplySettings />}
          {sec === 'webhook'     && <WebhookSettings />}
          {sec === 'notify'      && <NotifySettings />}
          {sec === 'gating'      && <GatingSettings />}
          {sec === 'about'       && <AboutSettings />}
          {!['theme','shortcuts','llm','tools','roles','concurrency','codeagent','selfupdate','backup','knowledge','security','asr','autosupply','webhook','notify','gating','about'].includes(sec) && (
            <div className="empty" style={{ height: '100%' }}>
              <Icon name={cur.ic} /><div>{cur.name}</div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
