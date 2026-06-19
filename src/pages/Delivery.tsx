import React, { useState, useEffect, useCallback, useRef } from 'react';
import { createPortal } from 'react-dom';
import Icon from '../components/Icon';
import Select from '../components/Select';
import Toast, { type ToastData } from '../components/Toast';
import {
  listActiveProjects, listPrototypePrompts, generatePrototypePrompt, deletePrototypePrompt, updatePrototypePrompt,
  openUrl, launchOpenDesign,
  listSecurityAudits, listDeployments, generateDeployScript, confirmDeploy, updateDeployScript, deleteDeployment,
  runProactiveScan, listTestSessions, getWidgetSnippet, getWebhookStatus,
  listDeliveryArtifacts, importDeliveryArtifact, deleteDeliveryArtifact, revealDeliveryArtifact, renameDeliveryArtifact,
  type Project, type PrototypePrompt, type SecurityAudit, type Deployment, type TestSession,
  type DeliveryArtifact, type WebhookStatus,
} from '../services';

const ART_NODE_OPTS = [
  { value: 'requirement', label: '需求' }, { value: 'materials', label: '物料' },
  { value: 'prototype', label: '原型' }, { value: 'spec', label: 'Spec' },
  { value: 'code', label: '编码' }, { value: 'test', label: '测试' },
  { value: 'security', label: '安全' }, { value: 'deploy', label: '部署' },
  { value: 'general', label: '通用' },
];
const ART_NODE_LABEL: Record<string, string> = Object.fromEntries(ART_NODE_OPTS.map(o => [o.value, o.label]));
function fmtSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

// 数出项目 config_yaml 中已配置、巡检会真正执行的检查项（与后端 configured_checks 同义：
// test.unit / test.integration / quality.{lint,typing,security}）。0 = 巡检会直接跳过。
function countConfiguredChecks(configYaml: string | null): number {
  if (!configYaml) return 0;
  let section = '';
  let count = 0;
  for (const raw of configYaml.split('\n')) {
    if (!raw.trim() || raw.trimStart().startsWith('#')) continue;
    const indent = raw.length - raw.trimStart().length;
    const line = raw.trim();
    if (indent === 0 && line.endsWith(':')) { section = line.slice(0, -1); continue; }
    if (section === 'test' && /^(unit|integration)\s*:/.test(line)) count++;
    else if (section === 'quality' && /^(lint|typing|security)\s*:\s*\S/.test(line)) count++;
  }
  return count;
}

// 项目是否配了 deploy/preview 命令——没有时部署只能生成通用 git 骨架脚本。
function hasDeployConfig(configYaml: string | null): boolean {
  return !!configYaml && /^(deploy|preview)\s*:/m.test(configYaml);
}

// 前置条件提示条：点明该交付功能"能真正跑起来"的条件，避免空态被误认为假数据。
// Inline hint shown as an icon next to a panel title; the note text appears on
// hover (native title tooltip — plain text only, no markup).
function InfoHint({ text, variant = 'info' }: { text: string; variant?: 'info' | 'warn' | 'ok' }) {
  const map = {
    info: { icon: 'alert', color: 'var(--text-faint)' },
    warn: { icon: 'alert', color: 'var(--amber)' },
    ok: { icon: 'check', color: 'var(--green)' },
  } as const;
  const m = map[variant];
  return (
    <span title={text} style={{ display: 'inline-flex', alignItems: 'center', cursor: 'help', color: m.color }}>
      <Icon name={m.icon} size={14} />
    </span>
  );
}

const codeBlock: React.CSSProperties = {
  margin: 0, fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-2)',
  whiteSpace: 'pre-wrap', maxHeight: 160, overflow: 'auto', background: 'var(--code-bg)', borderRadius: 8, padding: '8px 10px',
};
const inputStyle: React.CSSProperties = {
  background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 9,
  padding: '8px 10px', color: 'var(--text)', fontFamily: 'var(--font-sans)', fontSize: 'var(--text-control)',
};

const SEV_CHIP: Record<string, string> = {
  critical: 'red', high: 'red', medium: 'amber', low: 'blue', none: 'green',
};
const AUDIT_CHIP: Record<string, string> = {
  passed: 'green', flagged: 'amber', failed: 'red', running: 'blue',
};
const DEPLOY_CHIP: Record<string, string> = {
  deployed: 'green', failed: 'red', running: 'amber', pending_confirm: 'blue',
};
const TOOL_OPTS = [
  { value: 'generic', label: '通用设计工具' },
  { value: 'opendesign', label: 'OpenDesign' },
  { value: 'stitch', label: 'Google Stitch' },
  { value: 'claude_design', label: 'Claude Design' },
];
const ENV_OPTS = [
  { value: 'production', label: '生产环境' },
  { value: 'staging', label: '预发环境' },
];
// 复制按钮旁的「在设计工具打开」下拉：主流 AI 原型/设计站点，在系统浏览器打开供直接粘贴提示词。
const DESIGN_SITES = [
  { id: 'v0', label: 'v0', url: 'https://v0.app' },
  { id: 'lovable', label: 'Lovable', url: 'https://lovable.dev' },
  { id: 'stitch', label: 'Stitch', url: 'https://stitch.withgoogle.com' },
  { id: 'claude', label: 'Claude Design', url: 'https://claude.ai/design' },
];
// 下拉项 = 站点跳转 + OpenDesign 本地服务拉起（值 'opendesign' 特判）。
const DESIGN_TOOL_OPTS = [
  ...DESIGN_SITES.map(s => ({ value: s.id, label: s.label })),
  { value: 'opendesign', label: 'OpenDesign（拉起本地）' },
];

function ts(s: string | null): string {
  if (!s) return '—';
  return s.replace('T', ' ').slice(0, 19);
}

type StageId = 'design' | 'release' | 'operate' | 'archive';

function ConfirmModal({ msg, onOk, onCancel }: { msg: string; onOk: () => void; onCancel: () => void }) {
  return createPortal(
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', display: 'grid', placeItems: 'center', zIndex: 9999 }} onClick={onCancel}>
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

export default function Delivery() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectId, setProjectId] = useState('');
  const [prototypes, setPrototypes] = useState<PrototypePrompt[]>([]);
  const [audits, setAudits] = useState<SecurityAudit[]>([]);
  const [deploys, setDeploys] = useState<Deployment[]>([]);
  const [scans, setScans] = useState<TestSession[]>([]);
  const [toolTarget, setToolTarget] = useState('generic');
  const [targetEnv, setTargetEnv] = useState('production');
  const [busy, setBusy] = useState('');
  const [toast, setToast] = useState<ToastData | null>(null);
  const showError = useCallback((msg: string) => setToast({ msg, tone: 'error' }), []);
  const [stage, setStage] = useState<StageId>('design');
  const [openDeploy, setOpenDeploy] = useState<string>('');
  const [snippet, setSnippet] = useState('');

  const [editProto, setEditProto] = useState<{ id: string; title: string; prompt: string } | null>(null);
  const [editDeploy, setEditDeploy] = useState<{ id: string; script: string } | null>(null);
  const [confirmDelDeploy, setConfirmDelDeploy] = useState<Deployment | null>(null);
  const [artifacts, setArtifacts] = useState<DeliveryArtifact[]>([]);
  const [artNode, setArtNode] = useState('prototype');
  const [confirmDelArt, setConfirmDelArt] = useState<DeliveryArtifact | null>(null);
  const [editArt, setEditArt] = useState<{ id: string; name: string } | null>(null);
  const [webhook, setWebhook] = useState<WebhookStatus | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    listActiveProjects()
      .then(ps => {
        setProjects(ps);
        if (ps.length && !projectId) setProjectId(ps[0].id);
      })
      .catch(() => setProjects([]));
    // Webhook 是全局服务（非按项目），用于判断反馈 Widget 嵌入脚本是否可达。
    getWebhookStatus().then(setWebhook).catch(() => setWebhook(null));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const load = useCallback(async (pid: string) => {
    if (!pid) return;
    try {
      const [pp, sa, dp, tsx, ar] = await Promise.all([
        listPrototypePrompts(pid).catch(() => []),
        listSecurityAudits(pid).catch(() => []),
        listDeployments(pid).catch(() => []),
        listTestSessions(pid).catch(() => []),
        listDeliveryArtifacts(pid).catch(() => []),
      ]);
      setPrototypes(pp);
      setAudits(sa);
      setDeploys(dp);
      setScans(tsx.filter(s => s.session_type === 'proactive'));
      setArtifacts(ar);
    } catch (e) { showError(String(e)); }
  }, [showError]);

  // The widget snippet is deterministic (derived from projectId + webhook config),
  // so auto-generate it on project change / mount instead of keeping it in ephemeral
  // state — otherwise a full page refresh would wipe it and force a manual re-click.
  useEffect(() => {
    load(projectId);
    if (projectId) getWidgetSnippet(projectId).then(setSnippet).catch(() => setSnippet(''));
    else setSnippet('');
  }, [projectId, load]);

  const run = async (key: string, fn: () => Promise<unknown>) => {
    setBusy(key);
    try { await fn(); await load(projectId); }
    catch (e) { showError(String(e)); }
    finally { setBusy(''); }
  };

  const copy = async (text: string) => {
    try {
      await navigator.clipboard?.writeText(text);
      setToast({ msg: '已复制到剪贴板', tone: 'success' });
    } catch { showError('复制失败，请手动选择文本复制'); }
  };

  // 在系统浏览器打开主流设计站点（先复制提示词，再跳转，方便直接粘贴）。
  const openSite = async (url: string) => {
    try { await openUrl(url); }
    catch (e) { showError(String(e)); }
  };

  // 下拉分发：OpenDesign 走本地服务拉起，其余为站点浏览器跳转。
  const handleDesignTool = (id: string) => {
    if (id === 'opendesign') { launchOD(); return; }
    const site = DESIGN_SITES.find(s => s.id === id);
    if (site) openSite(site.url);
  };

  // 深度支持 OpenDesign：拉起本地服务（detached），就绪后在浏览器打开。
  const launchOD = async () => {
    setBusy('opendesign');
    setToast({ msg: '正在拉起本地 OpenDesign 服务…', tone: 'info' });
    try {
      const url = await launchOpenDesign();
      await openUrl(url);
      setToast({ msg: `已打开 OpenDesign（${url}）`, tone: 'success' });
    } catch (e) { showError(String(e)); }
    finally { setBusy(''); }
  };

  const onUpload = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) {
      setBusy('upload');
      const reader = new FileReader();
      reader.onload = async () => {
        try {
          const b64 = String(reader.result).split(',')[1] ?? '';
          await importDeliveryArtifact(projectId, artNode, file.name, file.type || 'application/octet-stream', b64);
          await load(projectId);
        } catch (x) { showError(String(x)); }
        finally { setBusy(''); }
      };
      reader.readAsDataURL(file);
    }
    e.target.value = '';
  };
  const reveal = async (a: DeliveryArtifact) => {
    try {
      await revealDeliveryArtifact(a.id);
    } catch (x) { showError(String(x)); }
  };
  const saveArtName = async () => {
    if (!editArt) return;
    const name = editArt.name.trim();
    const prev = artifacts.find(a => a.id === editArt.id)?.original_name;
    if (!name || name === prev) { setEditArt(null); return; }
    const target = editArt.id;
    setEditArt(null);
    try {
      await renameDeliveryArtifact(target, name);
      await load(projectId);
    } catch (x) { showError(String(x)); }
  };

  const activeProject = projects.find(p => p.id === projectId) ?? null;
  const checkCount = countConfiguredChecks(activeProject?.config_yaml ?? null);
  const deployConfigured = hasDeployConfig(activeProject?.config_yaml ?? null);
  const webhookReachable = webhook?.running === true;

  const totalArtSize = artifacts.reduce((sum, a) => sum + a.size_bytes, 0);
  const OVERVIEW: { id: StageId; label: string; ic: string; color: string; count: number; delta: string }[] = [
    { id: 'design',  label: '设计提示词', ic: 'palette',     color: 'var(--violet)', count: prototypes.length, delta: prototypes[0] ? `最新 ${prototypes[0].tool_target}` : '尚未生成' },
    { id: 'release', label: '部署记录',   ic: 'cloudUpload', color: 'var(--blue)',   count: deploys.length,    delta: deploys[0] ? `最新 ${deploys[0].status.replace(/_/g, ' ')}` : (audits[0] ? `安全 ${audits[0].status}` : '尚未发布') },
    { id: 'operate', label: '巡检记录',   ic: 'flask',       color: 'var(--ember)',  count: scans.length,      delta: scans[0] ? `最新 ${scans[0].status}` : '尚未巡检' },
    { id: 'archive', label: '交付产物',   ic: 'package',     color: 'var(--green)',  count: artifacts.length,  delta: artifacts.length ? fmtSize(totalArtSize) : '暂无产物' },
  ];

  return (
    <div className="content">
      <div className="audit-top" style={{ height: 56 }}>
        <div className="eyebrow" style={{ fontSize: 'var(--text-heading)' }}>
          <span className="en">DELIVERY</span><span className="cn">· 交付流水线</span>
        </div>
        <button className="btn btn-sm" style={{ marginLeft: 'auto' }} onClick={() => load(projectId)}><Icon name="refresh" size={15} />刷新</button>
      </div>

      {/* 左右结构：左侧项目列表 + 右侧交付内容 */}
      <div className="set-wrap">

        {/* 左：在产项目列表 */}
        <div className="set-nav" style={{ width: 220, overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: 4 }}>
          {projects.length === 0
            ? <div style={{ padding: '24px 12px', textAlign: 'center', fontSize: 'var(--text-label)', color: 'var(--text-faint)' }}>暂无在产项目</div>
            : projects.map(p => {
                const active = projectId === p.id;
                return (
                  <div key={p.id} className={'set-nav-item' + (active ? ' active' : '')} onClick={() => setProjectId(p.id)}
                    style={{ flexDirection: 'column', alignItems: 'flex-start', gap: 3, padding: '9px 12px' }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8, width: '100%' }}>
                      <div style={{ width: 26, height: 26, borderRadius: 7, background: active ? 'var(--ember)' : 'var(--bg-3)', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 'var(--text-label)', fontWeight: 800, color: active ? '#fff' : 'var(--text-3)', flexShrink: 0, fontFamily: 'var(--font-display)' }}>
                        {p.name[0]}
                      </div>
                      <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: 'var(--text-body)', fontWeight: 600 }}>{p.name}</span>
                    </div>
                    <div style={{ paddingLeft: 34, fontSize: 'var(--text-caption)', color: active ? 'var(--ember-soft)' : 'var(--text-faint)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', width: '100%' }}>
                      {p.description || p.slug}
                    </div>
                  </div>
                );
              })}
        </div>

        {/* 右：交付内容 */}
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden', minWidth: 0 }}>
        <div className="scroll" style={{ flex: 1, minWidth: 0, overflow: 'auto', background: 'var(--bg)' }}>
        <div className="rise" style={{ maxWidth: 1280, width: '100%', margin: '0 auto', padding: '22px 24px', minHeight: '100%', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', gap: 16 }}>
        {!projectId && <div className="empty-compact" style={{ padding: '24px 18px' }}>请从左侧选择一个在产项目</div>}

        {projectId && <>
          {/* 概览即导航：四个交付阶段的实时统计卡同时作为阶段切换器，点击切换当前阶段 */}
          <div className="stat-grid" style={{ marginBottom: 0 }}>
            {OVERVIEW.map(o => {
              const on = stage === o.id;
              return (
                <button key={o.id} className="stat" onClick={() => setStage(o.id)} aria-pressed={on}
                  style={{ textAlign: 'left', cursor: 'pointer', transition: 'border-color .12s, background .12s, box-shadow .12s',
                    border: on ? '1px solid var(--ember)' : '1px solid var(--border)',
                    background: on ? 'var(--ember-tint)' : 'var(--bg-2)',
                    boxShadow: on ? '0 0 0 3px var(--ember-tint)' : 'none' }}>
                  <div className="stat-ic" style={{ background: `color-mix(in oklab, ${o.color} 16%, transparent)`, color: o.color }}><Icon name={o.ic} size={18} /></div>
                  <div className="stat-main">
                    <div className="stat-label">{o.label}</div>
                    <div className="stat-val">{o.count}</div>
                  </div>
                  <div className="stat-delta">{o.delta}</div>
                </button>
              );
            })}
          </div>

          <div className="del-stage">
          {/* 设计原型 */}
          {stage === 'design' && (
          <div className="panel">
            <div className="panel-head">
              <div className="panel-title">
                <Icon name="palette" size={17} style={{ color: 'var(--violet)' }} />原型设计提示词
                <InfoHint text="提示词由项目根目录的 DESIGN.md 与技术规格喂给设计角色生成；仓库内有 DESIGN.md 时质量最佳，缺失时退回通用启发式模板。" />
              </div>
              <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                <div style={{ minWidth: 150 }}>
                  <Select className="sm" value={toolTarget} onChange={setToolTarget} options={TOOL_OPTS} />
                </div>
                <button className="btn btn-primary btn-sm" disabled={busy === 'proto'}
                  onClick={() => run('proto', () => generatePrototypePrompt(projectId, null, toolTarget))}>
                  <Icon name="zap" size={14} />{busy === 'proto' ? '生成中…' : '生成设计提示词'}
                </button>
              </div>
            </div>
            {prototypes.length === 0
              ? <div className="empty-compact" style={{ padding: '18px' }}>暂无设计提示词</div>
              : prototypes.map(p => (
                <div key={p.id} style={{ padding: '12px 16px', borderTop: '1px solid var(--border)', display: 'flex', flexDirection: 'column', flex: '1 1 0', minHeight: 180 }}>
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, marginBottom: 6, flexShrink: 0 }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
                      <span className="chip violet">{p.tool_target}</span>
                      <span style={{ fontSize: 'var(--text-title)', fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{p.title}</span>
                    </div>
                    <div style={{ display: 'flex', gap: 6, flexShrink: 0, alignItems: 'center', justifyContent: 'flex-end' }}>
                      <button className="btn btn-sm" onClick={() => copy(p.prompt)}><Icon name="copy" size={13} />复制</button>
                      <div style={{ minWidth: 150 }}>
                        <Select className="sm" value="" placeholder={busy === 'opendesign' ? '启动中…' : '在设计工具打开 ↗'}
                          options={DESIGN_TOOL_OPTS} onChange={handleDesignTool} />
                      </div>
                      <button className="btn btn-sm" onClick={() => setEditProto(editProto?.id === p.id ? null : { id: p.id, title: p.title, prompt: p.prompt })}><Icon name="edit" size={13} />完善</button>
                      <button className="btn btn-sm btn-danger" disabled={busy === 'delproto' + p.id} onClick={() => run('delproto' + p.id, async () => { await deletePrototypePrompt(p.id); if (editProto?.id === p.id) setEditProto(null); })}><Icon name="trash" size={13} /></button>
                    </div>
                  </div>
                  {editProto?.id === p.id
                    ? <div style={{ display: 'flex', flexDirection: 'column', gap: 6, flex: 1, minHeight: 0 }}>
                        <input value={editProto.title} onChange={e => setEditProto(s => s && { ...s, title: e.target.value })} style={inputStyle} />
                        <textarea value={editProto.prompt} onChange={e => setEditProto(s => s && { ...s, prompt: e.target.value })} style={{ ...inputStyle, flex: 1, minHeight: 130, fontFamily: 'var(--font-mono)', resize: 'none' }} />
                        <div style={{ display: 'flex', gap: 6, justifyContent: 'flex-end', flexShrink: 0 }}>
                          <button className="btn btn-sm" onClick={() => setEditProto(null)}>取消</button>
                          <button className="btn btn-sm btn-primary" disabled={busy === 'saveproto'} onClick={() => run('saveproto', async () => { await updatePrototypePrompt(editProto.id, editProto.title, editProto.prompt); setEditProto(null); })}><Icon name="check" size={13} />保存</button>
                        </div>
                      </div>
                    : <pre style={{ ...codeBlock, flex: 1, minHeight: 0, maxHeight: 'none' }}>{p.prompt}</pre>}
                </div>
              ))}
          </div>
          )}

          {/* 安全发布：安全审查 + 部署上线 */}
          {stage === 'release' && <>
          <div className="panel">
            <div className="panel-head">
              <div className="panel-title">
                <Icon name="shield" size={17} style={{ color: 'var(--green)' }} />安全审查
                {audits.length === 0 && (
                  <InfoHint text="安全审查会在每次 CR 合并时自动运行（扫描合并 diff 的密钥/危险操作）。该项目还没有合并记录，故暂无结果——无需手动触发。" />
                )}
              </div>
              <span className="sec-kicker">合并时自动运行</span>
            </div>
            {audits.length === 0
              ? <div className="empty-compact" style={{ padding: '18px' }}>暂无安全审查记录</div>
              : audits.map(a => (
                <div key={a.id} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, padding: '11px 16px', borderTop: '1px solid var(--border)' }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 10, minWidth: 0 }}>
                    <span className={'chip ' + (AUDIT_CHIP[a.status] || '')}>{a.status}</span>
                    <span className={'chip ' + (SEV_CHIP[a.severity] || 'green')}>{a.severity}</span>
                    <span style={{ fontSize: 'var(--text-control)', color: 'var(--text-2)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{a.summary}</span>
                  </div>
                  <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>{ts(a.completed_at || a.started_at)}</span>
                </div>
              ))}
          </div>

          <div className="panel">
            <div className="panel-head">
              <div className="panel-title">
                <Icon name="cloudUpload" size={17} style={{ color: 'var(--blue)' }} />部署上线
                {!deployConfigured && (
                  <InfoHint variant="warn" text="项目未配置 deploy / preview 命令，生成的脚本只是通用 git 拉取骨架。建议在「项目管理 → 运行配置」补充 preview.build / preview.start 或 deploy.command，脚本才会贴合真实部署流程。" />
                )}
              </div>
              <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                <div style={{ minWidth: 130 }}>
                  <Select className="sm" value={targetEnv} onChange={setTargetEnv} options={ENV_OPTS} />
                </div>
                <button className="btn btn-primary btn-sm" disabled={busy === 'deploy'}
                  onClick={() => run('deploy', () => generateDeployScript(projectId, targetEnv))}>
                  <Icon name="package" size={14} />{busy === 'deploy' ? '生成中…' : '生成部署脚本'}
                </button>
              </div>
            </div>
            {deploys.length === 0
              ? <div className="empty-compact" style={{ padding: '18px' }}>暂无部署记录</div>
              : deploys.map(d => (
                <div key={d.id} style={{ padding: '11px 16px', borderTop: '1px solid var(--border)' }}>
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8 }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 10, minWidth: 0 }}>
                      <span className={'chip ' + (DEPLOY_CHIP[d.status] || '')}>{d.status.replace(/_/g, ' ')}</span>
                      <span className="chip">{d.target_env}</span>
                      <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>{ts(d.created_at)}</span>
                    </div>
                    <div style={{ display: 'flex', gap: 6 }}>
                      <button className="btn btn-sm" onClick={() => setOpenDeploy(openDeploy === d.id ? '' : d.id)}>
                        <Icon name={openDeploy === d.id ? 'eye-off' : 'eye'} size={13} />{openDeploy === d.id ? '收起' : '查看'}
                      </button>
                      {d.status === 'pending_confirm' && (
                        <button className="btn btn-sm"
                          onClick={() => {
                            setOpenDeploy(d.id);
                            setEditDeploy(editDeploy?.id === d.id ? null : { id: d.id, script: d.script });
                          }}>
                          <Icon name="edit" size={13} />{editDeploy?.id === d.id ? '取消编辑' : '编辑'}
                        </button>
                      )}
                      {d.status !== 'running' && (
                        <button className="btn btn-sm btn-danger" disabled={busy === 'deldeploy' + d.id}
                          onClick={() => setConfirmDelDeploy(d)}>
                          <Icon name="trash" size={13} />
                        </button>
                      )}
                      {d.status === 'pending_confirm' && (
                        <button className="btn btn-primary btn-sm" disabled={busy === 'confirm' + d.id}
                          onClick={() => run('confirm' + d.id, () => confirmDeploy(d.id))}>
                          <Icon name="play" size={13} />{busy === 'confirm' + d.id ? '部署中…' : '确认部署'}
                        </button>
                      )}
                    </div>
                  </div>
                  {openDeploy === d.id && (
                    <div style={{ marginTop: 8 }}>
                      <div className="sec-kicker" style={{ marginBottom: 4 }}>部署脚本</div>
                      {editDeploy?.id === d.id
                        ? <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                            <textarea value={editDeploy.script} onChange={e => setEditDeploy(s => s && { ...s, script: e.target.value })}
                              style={{ ...inputStyle, minHeight: 160, fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', resize: 'vertical' }} />
                            <div style={{ display: 'flex', gap: 6, justifyContent: 'flex-end' }}>
                              <button className="btn btn-sm" onClick={() => setEditDeploy(null)}>取消</button>
                              <button className="btn btn-sm btn-primary" disabled={busy === 'savedeploy'}
                                onClick={() => run('savedeploy', async () => { await updateDeployScript(editDeploy.id, editDeploy.script); setEditDeploy(null); })}>
                                <Icon name="check" size={13} />保存
                              </button>
                            </div>
                          </div>
                        : <pre style={{ margin: 0, fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-2)', whiteSpace: 'pre-wrap', maxHeight: 160, overflow: 'auto', background: 'var(--code-bg)', borderRadius: 8, padding: '8px 10px' }}>{d.script}</pre>}
                      {d.log && <>
                        <div className="sec-kicker" style={{ margin: '8px 0 4px' }}>执行日志</div>
                        <pre style={{ margin: 0, fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-2)', whiteSpace: 'pre-wrap', maxHeight: 160, overflow: 'auto', background: 'var(--code-bg)', borderRadius: 8, padding: '8px 10px' }}>{d.log}</pre>
                      </>}
                    </div>
                  )}
                </div>
              ))}
          </div>
          </>}

          {/* 运维监控：主动巡检 + 反馈 Widget */}
          {stage === 'operate' && <>
          <div className="panel">
            <div className="panel-head">
              <div className="panel-title">
                <Icon name="flask" size={17} style={{ color: 'var(--ember)' }} />主动巡检
                {checkCount === 0
                  ? <InfoHint variant="warn" text="项目未配置任何检查命令，点击巡检会直接跳过、不会产生有意义的结果。请在「项目管理 → 运行配置」的 test.unit / test.integration / quality.lint / quality.typing / quality.security 中至少填一条命令。" />
                  : <InfoHint variant="ok" text={`已配置 ${checkCount} 项检查命令，巡检会在主仓库依次执行，失败项自动建为需求重进流水线。`} />}
              </div>
              <button className="btn btn-sm" disabled={busy === 'scan'}
                onClick={() => {
                  if (checkCount === 0) {
                    setToast({ msg: '项目未配置任何检查命令，无需巡检。请先在「项目管理 → 运行配置」的 test / quality 中至少填写一条检查命令。', tone: 'info' });
                    return;
                  }
                  run('scan', () => runProactiveScan(projectId));
                }}>
                <Icon name="refresh" size={14} />{busy === 'scan' ? '巡检中…' : '立即巡检'}
              </button>
            </div>
            {scans.length === 0
              ? <div className="empty-compact" style={{ padding: '18px' }}>暂无巡检记录</div>
              : scans.map(s => (
                <div key={s.id} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, padding: '11px 16px', borderTop: '1px solid var(--border)' }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 10, minWidth: 0 }}>
                    <span className={'chip ' + (s.status === 'passed' ? 'green' : s.status === 'failed' ? 'red' : 'blue')}>{s.status}</span>
                    <span className="chip">{s.trigger}</span>
                    <span style={{ fontSize: 'var(--text-control)', color: 'var(--text-2)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{s.summary}</span>
                  </div>
                  <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>{ts(s.completed_at || s.started_at)}</span>
                </div>
              ))}
          </div>

          <div className="panel">
            <div className="panel-head">
              <div className="panel-title">
                <Icon name="code" size={17} style={{ color: 'var(--blue)' }} />反馈 Widget
                {!webhookReachable
                  ? <InfoHint variant="warn" text="反馈 Widget 依赖本地 Webhook 服务，当前未运行，嵌入脚本里的 localhost 地址不可达。请在「设置 → 需求接收」启用 Webhook（需同时设置端口与 Token）。" />
                  : <InfoHint variant="ok" text={`Webhook 服务运行中（端口 ${webhook?.port}），嵌入脚本可正常接收反馈。`} />}
              </div>
              <button className="btn btn-sm" disabled={busy === 'widget'}
                onClick={() => run('widget', async () => { setSnippet(await getWidgetSnippet(projectId)); })}>
                <Icon name="code" size={14} />{busy === 'widget' ? '生成中…' : '生成嵌入代码'}
              </button>
            </div>
            {snippet
              ? <div style={{ padding: '12px 16px' }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 6 }}>
                    <span className="sec-kicker">将以下脚本粘贴到目标站点</span>
                    <button className="btn btn-sm" onClick={() => copy(snippet)}><Icon name="copy" size={13} />复制</button>
                  </div>
                  <pre style={codeBlock}>{snippet}</pre>
                </div>
              : <div className="empty-compact" style={{ padding: '18px' }}>点击「生成嵌入代码」获取该项目的反馈组件脚本</div>}
          </div>
          </>}

          {/* 交付归档：统一管理，持久化到 .autoforge/deliverables */}
          {stage === 'archive' && (
          <div className="panel">
            <div className="panel-head">
              <div className="panel-title">
                <Icon name="package" size={17} style={{ color: 'var(--green)' }} />交付产物
                <InfoHint text="持久化到项目仓库的 .autoforge/deliverables/<节点>/，统一管理全部交付材料。" />
              </div>
              <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                <div style={{ minWidth: 120 }}>
                  <Select className="sm" value={artNode} onChange={setArtNode} options={ART_NODE_OPTS} />
                </div>
                <button className="btn btn-primary btn-sm" disabled={busy === 'upload'} onClick={() => fileRef.current?.click()}>
                  <Icon name="upload" size={14} />{busy === 'upload' ? '上传中…' : '上传产物'}
                </button>
                <input ref={fileRef} type="file" style={{ display: 'none' }} onChange={onUpload} />
              </div>
            </div>
            {artifacts.length === 0
              ? <div className="empty-compact" style={{ padding: '14px 16px' }}>暂无交付产物</div>
              : artifacts.map(a => (
                <div key={a.id} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, padding: '10px 16px', borderTop: '1px solid var(--border)' }}>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 3, minWidth: 0 }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
                      <span className="chip">{ART_NODE_LABEL[a.node] || a.node}</span>
                      {editArt?.id === a.id
                        ? <input
                            autoFocus
                            value={editArt.name}
                            onChange={e => setEditArt(s => s && { ...s, name: e.target.value })}
                            onBlur={saveArtName}
                            onKeyDown={e => { if (e.key === 'Enter') saveArtName(); else if (e.key === 'Escape') setEditArt(null); }}
                            style={{ ...inputStyle, padding: '3px 8px', fontSize: 'var(--text-control)', fontWeight: 600, minWidth: 0, flex: 1 }}
                          />
                        : <span
                            onClick={() => setEditArt({ id: a.id, name: a.original_name })}
                            title="点击修改文件名"
                            style={{ fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', cursor: 'text' }}
                          >{a.original_name}</span>}
                      <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-faint)', flexShrink: 0 }}>{fmtSize(a.size_bytes)}</span>
                    </div>
                    <span title={a.rel_path} style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-faint)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{a.rel_path}</span>
                  </div>
                  <div style={{ display: 'flex', gap: 6 }}>
                    <button className="btn btn-sm" onClick={() => reveal(a)}><Icon name="folderOpen" size={13} />打开所在文件夹</button>
                    <button className="btn btn-sm btn-danger" disabled={busy === 'delart' + a.id} onClick={() => setConfirmDelArt(a)}><Icon name="trash" size={13} /></button>
                  </div>
                </div>
              ))}
          </div>
          )}
          </div>
        </>}
        </div>
        </div>
        </div>
      </div>
      {confirmDelArt && (
        <ConfirmModal
          msg={`确认删除交付产物「${confirmDelArt.original_name}」？此操作会同时删除磁盘文件，且不可恢复。`}
          onOk={() => { const a = confirmDelArt; setConfirmDelArt(null); run('delart' + a.id, async () => { await deleteDeliveryArtifact(a.id); }); }}
          onCancel={() => setConfirmDelArt(null)}
        />
      )}
      {confirmDelDeploy && (
        <ConfirmModal
          msg={`确认删除该部署记录（${confirmDelDeploy.target_env} · ${confirmDelDeploy.status.replace(/_/g, ' ')}）？此操作不可恢复。`}
          onOk={() => { const dep = confirmDelDeploy; setConfirmDelDeploy(null); run('deldeploy' + dep.id, async () => { await deleteDeployment(dep.id); if (editDeploy?.id === dep.id) setEditDeploy(null); if (openDeploy === dep.id) setOpenDeploy(''); }); }}
          onCancel={() => setConfirmDelDeploy(null)}
        />
      )}
      <Toast data={toast} onClose={() => setToast(null)} />
    </div>
  );
}
