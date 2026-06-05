import React, { useState, useEffect, useCallback } from 'react';
import Icon from '../components/Icon';
import Select from '../components/Select';
import {
  getIntakeConfig, updateIntakeConfig, getWebhookStatus,
  syncGithubIssues, runCodeScan, bulkImportIssues,
  listActiveProjects,
  type IntakeConfig, type SyncResult, type ScanResult, type BulkResult,
  type WebhookStatus, type Project,
} from '../services';

type Tab = 'webhook' | 'github' | 'scanner' | 'bulk';

// ── 内部小组件 ──────────────────────────────────────────────────────────────

function Card({ children, style }: { children: React.ReactNode; style?: React.CSSProperties }) {
  return (
    <div style={{
      background: 'var(--bg-2)',
      border: '1px solid var(--border-strong)',
      borderRadius: 14,
      padding: '18px 20px',
      ...style,
    }}>
      {children}
    </div>
  );
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div style={{
      fontSize: 11, fontWeight: 700, letterSpacing: '.07em',
      textTransform: 'uppercase', color: 'var(--text-faint)', marginBottom: 10,
    }}>
      {children}
    </div>
  );
}

function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <div
      role="switch" aria-checked={checked}
      onClick={() => onChange(!checked)}
      style={{
        width: 38, height: 22, borderRadius: 11, cursor: 'pointer', flexShrink: 0,
        background: checked ? 'var(--ember)' : 'var(--bg-3)',
        border: `1.5px solid ${checked ? 'var(--ember-deep)' : 'var(--border-strong)'}`,
        position: 'relative', transition: 'background .2s, border-color .2s',
      }}
    >
      <div style={{
        position: 'absolute', top: 2,
        left: checked ? 18 : 2,
        width: 14, height: 14, borderRadius: '50%',
        background: '#fff',
        transition: 'left .2s cubic-bezier(.4,0,.2,1)',
        boxShadow: '0 1px 3px rgba(0,0,0,.35)',
      }} />
    </div>
  );
}

function ResultBanner({ ok, children }: { ok?: boolean; children: React.ReactNode }) {
  return (
    <div style={{
      background: ok === false ? 'rgba(219,90,64,.1)' : 'rgba(79,157,107,.1)',
      border: `1px solid ${ok === false ? 'rgba(219,90,64,.3)' : 'rgba(79,157,107,.3)'}`,
      borderRadius: 10, padding: '10px 14px', fontSize: 12.5,
      color: ok === false ? 'var(--red)' : 'var(--green)',
      display: 'flex', alignItems: 'flex-start', gap: 8,
    }}>
      <Icon name={ok === false ? 'alert' : 'check'} size={14} style={{ flexShrink: 0, marginTop: 1 }} />
      <div>{children}</div>
    </div>
  );
}

function InfoBanner({ children }: { children: React.ReactNode }) {
  return (
    <div style={{
      background: 'rgba(139,122,216,.08)',
      border: '1px solid rgba(139,122,216,.22)',
      borderRadius: 10, padding: '10px 14px', fontSize: 12,
      color: 'var(--text-2)', display: 'flex', gap: 8,
    }}>
      <Icon name="bell" size={13} style={{ flexShrink: 0, marginTop: 1, color: 'var(--violet)' }} />
      <div>{children}</div>
    </div>
  );
}

function Toast({ msg, ok }: { msg: string; ok: boolean }) {
  if (!msg) return null;
  return (
    <div style={{
      position: 'fixed', bottom: 24, right: 24, zIndex: 500,
      background: ok ? 'var(--green)' : 'var(--red)',
      color: '#fff', borderRadius: 10, padding: '9px 16px', fontSize: 13,
      boxShadow: '0 4px 20px rgba(0,0,0,.4)', display: 'flex', alignItems: 'center', gap: 8,
    }}>
      <Icon name={ok ? 'check' : 'alert'} size={14} />
      {msg}
    </div>
  );
}

// ── Tab: Webhook ─────────────────────────────────────────────────────────────

function WebhookTab({ cfg, onChange }: {
  cfg: IntakeConfig;
  onChange: (updated: IntakeConfig) => void;
}) {
  const [form, setForm] = useState({
    enabled: cfg.webhook_enabled,
    port: String(cfg.webhook_port),
    token: cfg.webhook_token,
  });
  const [status, setStatus] = useState<WebhookStatus | null>(null);
  const [saving, setSaving] = useState(false);
  const [copied, setCopied] = useState(false);
  const [saveOk, setSaveOk] = useState<boolean | null>(null);

  useEffect(() => { getWebhookStatus().then(setStatus).catch(() => {}); }, [cfg]);

  const save = async () => {
    setSaving(true); setSaveOk(null);
    try {
      const updated = await updateIntakeConfig({
        webhook_enabled: form.enabled,
        webhook_port: parseInt(form.port) || 27182,
        webhook_token: form.token,
      });
      onChange(updated);
      setSaveOk(true);
      setTimeout(() => setSaveOk(null), 2500);
    } catch { setSaveOk(false); }
    finally { setSaving(false); }
  };

  const genToken = () => {
    const bytes = new Uint8Array(24);
    crypto.getRandomValues(bytes);
    const tok = btoa(String.fromCharCode(...bytes)).replace(/[+/=]/g, '');
    setForm(f => ({ ...f, token: tok }));
  };

  const curlExample = `curl -X POST http://127.0.0.1:${form.port}/webhook/issues \\
  -H "Authorization: Bearer ${form.token || '<token>'}" \\
  -H "Content-Type: application/json" \\
  -d '{"project_id":"<uuid>","title":"需求标题","description":"详细描述"}'`;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>

      {/* 状态卡 */}
      <Card>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 18 }}>
          <div style={{ width: 34, height: 34, borderRadius: 9, background: 'rgba(232,119,46,.15)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Icon name="zap" size={16} style={{ color: 'var(--ember)' }} />
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ fontWeight: 600, fontSize: 14 }}>HTTP Webhook 接口</div>
            <div style={{ fontSize: 11.5, color: 'var(--text-3)', marginTop: 1 }}>外部系统通过 POST 请求推送需求</div>
          </div>
          {status && (
            <span className={'chip ' + (status.running ? 'green' : '')}
              style={{ padding: '3px 10px', fontSize: 11 }}>
              <span style={{ width: 6, height: 6, borderRadius: '50%', background: status.running ? 'var(--green)' : 'var(--text-3)', display: 'inline-block', marginRight: 5 }} />
              {status.running ? `运行中 :${status.port}` : '已停止'}
            </span>
          )}
        </div>

        {/* 启用开关 */}
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '10px 14px', background: 'var(--bg-3)', borderRadius: 9, marginBottom: 14 }}>
          <div>
            <div style={{ fontWeight: 500, fontSize: 13 }}>启用 Webhook</div>
            <div style={{ fontSize: 11.5, color: 'var(--text-3)', marginTop: 1 }}>开启后外部系统可向本机端口推送需求</div>
          </div>
          <Toggle checked={form.enabled} onChange={v => setForm(f => ({ ...f, enabled: v }))} />
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
                placeholder="Bearer 令牌" style={{ flex: 1, fontFamily: 'var(--font-mono)', fontSize: 11.5, letterSpacing: '.03em' }} />
              <button className="btn btn-sm" onClick={genToken} title="随机生成 Token" style={{ flexShrink: 0 }}>
                <Icon name="refresh" size={13} />生成
              </button>
            </div>
          </div>
        </div>

        <InfoBanner>
          Webhook 仅监听 <code style={{ fontFamily: 'var(--font-mono)', fontSize: 11 }}>127.0.0.1</code>（本机），不暴露公网。
          更改端口或 Token 后点击「保存并应用」以重启服务。
        </InfoBanner>

        <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginTop: 14 }}>
          <button className="btn btn-primary" onClick={save} disabled={saving}>
            <Icon name="check" size={14} />{saving ? '保存中…' : '保存并应用'}
          </button>
          {saveOk === true && <span style={{ fontSize: 12, color: 'var(--green)' }}>✓ 已保存</span>}
          {saveOk === false && <span style={{ fontSize: 12, color: 'var(--red)' }}>保存失败</span>}
        </div>
      </Card>

      {/* curl 示例卡 */}
      <Card>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 10 }}>
          <SectionLabel>curl 示例</SectionLabel>
          <button className="icon-btn" style={{ width: 26, height: 26 }} title="复制"
            onClick={async () => { await navigator.clipboard.writeText(curlExample).catch(() => {}); setCopied(true); setTimeout(() => setCopied(false), 1200); }}>
            <Icon name={copied ? 'check' : 'copy'} size={13} />
          </button>
        </div>
        <pre style={{
          fontFamily: 'var(--font-mono)', fontSize: 11.5, lineHeight: 1.6,
          color: 'var(--text-2)', background: 'var(--bg-3)',
          borderRadius: 8, padding: '12px 14px',
          overflowX: 'auto', margin: 0, whiteSpace: 'pre-wrap', wordBreak: 'break-all',
          border: '1px solid var(--border)',
        }}>
          {curlExample}
        </pre>
      </Card>
    </div>
  );
}

// ── Tab: GitHub ───────────────────────────────────────────────────────────────

function GithubTab({ cfg, projects, onChange }: {
  cfg: IntakeConfig; projects: Project[];
  onChange: (updated: IntakeConfig) => void;
}) {
  const [form, setForm] = useState({
    owner: cfg.github_owner,
    repo: cfg.github_repo,
    token: cfg.github_token,
    project_id: cfg.github_project_id,
  });
  const [saving, setSaving] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [syncResult, setSyncResult] = useState<SyncResult | null>(null);
  const [syncErr, setSyncErr] = useState('');
  const [saveOk, setSaveOk] = useState<boolean | null>(null);

  const projectOpts = [
    { value: '', label: '请选择目标项目' },
    ...projects.map(p => ({ value: p.id, label: p.name })),
  ];

  const save = async () => {
    setSaving(true); setSaveOk(null);
    try {
      const updated = await updateIntakeConfig({
        github_owner: form.owner.trim(),
        github_repo: form.repo.trim(),
        github_token: form.token.trim(),
        github_project_id: form.project_id,
      });
      onChange(updated);
      setSaveOk(true);
      setTimeout(() => setSaveOk(null), 2500);
    } catch { setSaveOk(false); }
    finally { setSaving(false); }
  };

  const sync = async () => {
    setSyncing(true); setSyncResult(null); setSyncErr('');
    try {
      const r = await syncGithubIssues();
      setSyncResult(r);
    } catch (e) { setSyncErr(String(e)); }
    finally { setSyncing(false); }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>

      {/* 仓库配置卡 */}
      <Card>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 18 }}>
          <div style={{ width: 34, height: 34, borderRadius: 9, background: 'rgba(79,142,209,.15)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Icon name="code" size={16} style={{ color: 'var(--blue)' }} />
          </div>
          <div>
            <div style={{ fontWeight: 600, fontSize: 14 }}>GitHub Issues 同步</div>
            <div style={{ fontSize: 11.5, color: 'var(--text-3)', marginTop: 1 }}>单向拉取仓库 Issues，自动去重</div>
          </div>
        </div>

        <SectionLabel>仓库信息</SectionLabel>
        <div className="cfg-fields" style={{ gridTemplateColumns: '1fr 1fr', gap: 10, marginBottom: 14 }}>
          <div className="field" style={{ margin: 0 }}>
            <label>Owner / Org</label>
            <input value={form.owner} onChange={e => setForm(f => ({ ...f, owner: e.target.value }))} placeholder="your-org" />
          </div>
          <div className="field" style={{ margin: 0 }}>
            <label>Repository</label>
            <input value={form.repo} onChange={e => setForm(f => ({ ...f, repo: e.target.value }))} placeholder="my-project" />
          </div>
        </div>

        <SectionLabel>认证与目标</SectionLabel>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 10, marginBottom: 14 }}>
          <div className="field" style={{ margin: 0 }}>
            <label>GitHub Token（私有仓库必填，公开仓库可留空）</label>
            <input type="password" value={form.token} onChange={e => setForm(f => ({ ...f, token: e.target.value }))}
              placeholder="ghp_xxxxxxxxxxxx"
              style={{ fontFamily: 'var(--font-mono)', fontSize: 11.5 }} />
          </div>
          <div className="field" style={{ margin: 0 }}>
            <label>导入到 AutoForge 项目</label>
            <Select
              value={form.project_id}
              onChange={v => setForm(f => ({ ...f, project_id: v }))}
              options={projectOpts}
              placeholder="请选择目标项目"
            />
          </div>
        </div>

        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <button className="btn btn-primary" onClick={save} disabled={saving}>
            <Icon name="check" size={14} />{saving ? '保存中…' : '保存配置'}
          </button>
          {saveOk === true && <span style={{ fontSize: 12, color: 'var(--green)' }}>✓ 已保存</span>}
          {saveOk === false && <span style={{ fontSize: 12, color: 'var(--red)' }}>保存失败</span>}
        </div>
      </Card>

      {/* 同步操作卡 */}
      <Card>
        <SectionLabel>立即同步</SectionLabel>
        {cfg.github_last_sync && (
          <div style={{ fontSize: 12, color: 'var(--text-3)', marginBottom: 10 }}>
            上次同步：{new Date(cfg.github_last_sync).toLocaleString('zh-CN')}
          </div>
        )}
        {syncResult && (
          <ResultBanner ok>
            同步完成：导入 <strong>{syncResult.imported}</strong> 条，
            跳过重复 {syncResult.skipped} 条
            {syncResult.errors > 0 && `，错误 ${syncResult.errors} 条`}
          </ResultBanner>
        )}
        {syncErr && <ResultBanner ok={false}>{syncErr}</ResultBanner>}
        <button className="btn" style={{ marginTop: syncResult || syncErr ? 10 : 0 }}
          onClick={sync}
          disabled={syncing || !form.owner || !form.repo || !form.project_id}>
          <Icon name="refresh" size={14} style={{ animation: syncing ? 'spin 1s linear infinite' : undefined }} />
          {syncing ? '同步中…' : '立即同步'}
        </button>
      </Card>
    </div>
  );
}

// ── Tab: Scanner ──────────────────────────────────────────────────────────────

const SCAN_ITEMS = [
  {
    key: 'todo' as const,
    icon: 'code',
    color: '#8b7ad8',
    bg: 'rgba(139,122,216,.12)',
    title: 'TODO / FIXME 注释',
    desc: '扫描代码中的 TODO、FIXME、HACK、XXX 注释并作为 Debt 类需求入队',
  },
  {
    key: 'cargo' as const,
    icon: 'shield',
    color: '#db5a40',
    bg: 'rgba(219,90,64,.12)',
    title: 'cargo audit',
    desc: '检测 Rust 项目依赖中的已知安全漏洞（需 Cargo.lock）',
  },
  {
    key: 'npm' as const,
    icon: 'box',
    color: '#4f9d6b',
    bg: 'rgba(79,157,107,.12)',
    title: 'npm audit',
    desc: '检测 Node.js 项目依赖中的安全漏洞（需 package-lock.json）',
  },
];

function ScannerTab({ projects }: { projects: Project[] }) {
  const [projectId, setProjectId] = useState('');
  const [scanTypes, setScanTypes] = useState({ todo: true, cargo: true, npm: true });
  const [scanning, setScanning] = useState(false);
  const [result, setResult] = useState<ScanResult | null>(null);
  const [scanErr, setScanErr] = useState('');

  const projectOpts = [
    { value: '', label: '请选择目标项目' },
    ...projects.map(p => ({ value: p.id, label: p.name })),
  ];

  const scan = async () => {
    if (!projectId) return;
    setScanning(true); setResult(null); setScanErr('');
    const types: string[] = [];
    if (scanTypes.todo) types.push('todo');
    if (scanTypes.cargo) types.push('cargo_audit');
    if (scanTypes.npm) types.push('npm_audit');
    try {
      const r = await runCodeScan(projectId, types);
      setResult(r);
    } catch (e) { setScanErr(String(e)); }
    finally { setScanning(false); }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>

      {/* 项目选择卡 */}
      <Card>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 16 }}>
          <div style={{ width: 34, height: 34, borderRadius: 9, background: 'rgba(79,157,107,.15)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Icon name="search" size={16} style={{ color: 'var(--green)' }} />
          </div>
          <div>
            <div style={{ fontWeight: 600, fontSize: 14 }}>代码扫描</div>
            <div style={{ fontSize: 11.5, color: 'var(--text-3)', marginTop: 1 }}>主动发现代码库中的问题并自动入队</div>
          </div>
        </div>
        <div className="field" style={{ margin: 0 }}>
          <label>目标项目</label>
          <Select value={projectId} onChange={setProjectId} options={projectOpts} placeholder="请选择目标项目" />
        </div>
      </Card>

      {/* 扫描类型卡 */}
      <Card>
        <SectionLabel>扫描类型</SectionLabel>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          {SCAN_ITEMS.map(item => (
            <div
              key={item.key}
              onClick={() => setScanTypes(s => ({ ...s, [item.key]: !s[item.key] }))}
              style={{
                display: 'flex', alignItems: 'center', gap: 12, padding: '12px 14px',
                borderRadius: 10, cursor: 'pointer',
                background: scanTypes[item.key] ? item.bg : 'var(--bg-3)',
                border: `1px solid ${scanTypes[item.key] ? item.color + '44' : 'var(--border)'}`,
                transition: 'background .15s, border-color .15s',
              }}
            >
              <div style={{ width: 30, height: 30, borderRadius: 8, background: item.bg, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
                <Icon name={item.icon as any} size={14} style={{ color: item.color }} />
              </div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontWeight: 500, fontSize: 13 }}>{item.title}</div>
                <div style={{ fontSize: 11.5, color: 'var(--text-3)', marginTop: 2 }}>{item.desc}</div>
              </div>
              <div style={{
                width: 18, height: 18, borderRadius: 5, flexShrink: 0,
                background: scanTypes[item.key] ? item.color : 'transparent',
                border: `2px solid ${scanTypes[item.key] ? item.color : 'var(--border-strong)'}`,
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                transition: 'background .15s, border-color .15s',
              }}>
                {scanTypes[item.key] && <Icon name="check" size={10} style={{ color: '#fff' }} />}
              </div>
            </div>
          ))}
        </div>
      </Card>

      {/* 结果 + 操作 */}
      {result && (
        <ResultBanner ok>
          扫描完成：发现 <strong>{result.found}</strong> 处，
          新建 Issue <strong>{result.new_issues}</strong> 条
        </ResultBanner>
      )}
      {scanErr && <ResultBanner ok={false}>{scanErr}</ResultBanner>}
      <button className="btn btn-primary" style={{ alignSelf: 'flex-start' }}
        onClick={scan} disabled={scanning || !projectId}>
        <Icon name="search" size={14} style={{ animation: scanning ? 'spin 1s linear infinite' : undefined }} />
        {scanning ? '扫描中…' : '运行扫描'}
      </button>
    </div>
  );
}

// ── Tab: Bulk ─────────────────────────────────────────────────────────────────

const FORMAT_META = {
  text: { label: '纯文本', desc: '每行一个需求标题' },
  csv:  { label: 'CSV',   desc: '支持 title / description / category / severity 列' },
  json: { label: 'JSON',  desc: '对象数组，字段同 CSV' },
} as const;

const PLACEHOLDERS: Record<string, string> = {
  text: '每行一个需求标题，例如：\n添加用户头像上传功能\n优化搜索响应速度\n修复登录超时 Bug',
  csv: 'title,description,category,severity\n添加头像上传,用户可上传个人头像,Feature,medium\n修复登录 Bug,,Bug,high',
  json: '[\n  { "title": "需求1", "description": "描述", "category": "Feature", "severity": "medium" },\n  { "title": "需求2" }\n]',
};

function BulkTab({ projects }: { projects: Project[] }) {
  const [projectId, setProjectId] = useState('');
  const [format, setFormat] = useState<'text' | 'csv' | 'json'>('text');
  const [content, setContent] = useState('');
  const [importing, setImporting] = useState(false);
  const [result, setResult] = useState<BulkResult | null>(null);
  const [importErr, setImportErr] = useState('');

  const projectOpts = [
    { value: '', label: '请选择目标项目' },
    ...projects.map(p => ({ value: p.id, label: p.name })),
  ];

  const doImport = async () => {
    if (!projectId || !content.trim()) return;
    setImporting(true); setResult(null); setImportErr('');
    try {
      const r = await bulkImportIssues(projectId, format, content);
      setResult(r);
    } catch (e) { setImportErr(String(e)); }
    finally { setImporting(false); }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>

      {/* 配置卡 */}
      <Card>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 18 }}>
          <div style={{ width: 34, height: 34, borderRadius: 9, background: 'rgba(224,163,46,.15)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Icon name="arrowUp" size={16} style={{ color: 'var(--amber)' }} />
          </div>
          <div>
            <div style={{ fontWeight: 600, fontSize: 14 }}>批量导入</div>
            <div style={{ fontSize: 11.5, color: 'var(--text-3)', marginTop: 1 }}>粘贴或上传多条需求，一次性入队分析（最多 200 条）</div>
          </div>
        </div>

        <div className="field" style={{ margin: '0 0 14px' }}>
          <label>目标项目</label>
          <Select value={projectId} onChange={setProjectId} options={projectOpts} placeholder="请选择目标项目" />
        </div>

        <SectionLabel>格式</SectionLabel>
        <div style={{ display: 'flex', gap: 6, marginBottom: 14 }}>
          {(Object.entries(FORMAT_META) as [keyof typeof FORMAT_META, { label: string; desc: string }][]).map(([key, meta]) => (
            <button key={key}
              className={'btn btn-sm' + (format === key ? ' btn-primary' : '')}
              onClick={() => { setFormat(key); setContent(''); }}
              style={{ flexDirection: 'column', alignItems: 'flex-start', padding: '8px 12px', height: 'auto', gap: 2 }}>
              <span style={{ fontWeight: 600, fontSize: 12 }}>{meta.label}</span>
              <span style={{ fontSize: 10.5, color: format === key ? 'rgba(255,255,255,.75)' : 'var(--text-2)', fontWeight: 400 }}>{meta.desc}</span>
            </button>
          ))}
        </div>
      </Card>

      {/* 内容区卡 */}
      <Card>
        <SectionLabel>内容（{content.split('\n').filter(l => l.trim()).length} 行）</SectionLabel>
        <textarea
          value={content}
          onChange={e => setContent(e.target.value)}
          placeholder={PLACEHOLDERS[format]}
          rows={10}
          style={{
            width: '100%', boxSizing: 'border-box',
            fontFamily: 'var(--font-mono)', fontSize: 12, lineHeight: 1.6,
            background: 'var(--bg-3)', border: '1px solid var(--border)',
            borderRadius: 8, padding: '10px 12px', color: 'var(--text)',
            resize: 'vertical', outline: 'none',
          }}
        />
      </Card>

      {/* 结果 */}
      {result && (
        <ResultBanner ok={result.errors.length === 0}>
          <div>导入完成：共 {result.total} 条，成功 <strong>{result.imported}</strong>，跳过 {result.skipped}</div>
          {result.errors.length > 0 && (
            <div style={{ marginTop: 6 }}>
              {result.errors.slice(0, 5).map((e, i) => <div key={i} style={{ marginTop: 2 }}>• {e}</div>)}
              {result.errors.length > 5 && <div>…及 {result.errors.length - 5} 条其他错误</div>}
            </div>
          )}
        </ResultBanner>
      )}
      {importErr && <ResultBanner ok={false}>{importErr}</ResultBanner>}

      <button className="btn btn-primary" style={{ alignSelf: 'flex-start' }}
        onClick={doImport} disabled={importing || !projectId || !content.trim()}>
        <Icon name="arrowUp" size={14} />{importing ? '导入中…' : '开始导入'}
      </button>
    </div>
  );
}

// ── IntakePage ────────────────────────────────────────────────────────────────

const NAV_ITEMS: { id: Tab; label: string; desc: string; ic: string }[] = [
  { id: 'webhook', label: 'Webhook',  desc: '外部系统推送需求',   ic: 'zap' },
  { id: 'github',  label: 'GitHub',   desc: 'Issues 自动同步',    ic: 'code' },
  { id: 'scanner', label: '代码扫描', desc: 'TODO · 安全漏洞',    ic: 'search' },
  { id: 'bulk',    label: '批量导入', desc: 'CSV · JSON · 文本',  ic: 'arrowUp' },
];

export default function IntakePage() {
  const [tab, setTab] = useState<Tab>('webhook');
  const [cfg, setCfg] = useState<IntakeConfig | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [error, setError] = useState('');
  const [toast, setToast] = useState({ msg: '', ok: true });

  const showToast = useCallback((msg: string, ok = true) => {
    setToast({ msg, ok });
    setTimeout(() => setToast({ msg: '', ok: true }), 3000);
  }, []);

  useEffect(() => {
    Promise.all([getIntakeConfig(), listActiveProjects()])
      .then(([c, ps]) => { setCfg(c); setProjects(ps); })
      .catch(e => setError(String(e)));
  }, []);

  return (
    <div className="content">
      {/* 顶栏 */}
      <div className="audit-top" style={{ height: 56 }}>
        <div className="eyebrow" style={{ fontSize: 17 }}>
          <span className="en">INTAKE</span><span className="cn">· 需求入口</span>
        </div>
      </div>

      {/* 左右布局 */}
      <div className="set-wrap">
        {/* 左侧导航 */}
        <div className="set-nav">
          {NAV_ITEMS.map(it => (
            <div key={it.id}
              className={'set-nav-item' + (tab === it.id ? ' active' : '')}
              onClick={() => setTab(it.id)}
            >
              <Icon name={it.ic as any} size={18} />
              <div style={{ minWidth: 0 }}>
                <div>{it.label}</div>
                <div style={{ fontSize: 11, color: tab === it.id ? 'var(--ember-soft)' : 'var(--text-faint)', fontWeight: 400, marginTop: 1 }}>
                  {it.desc}
                </div>
              </div>
            </div>
          ))}
        </div>

        {/* 右侧内容 */}
        <div className="set-body scroll" style={{ flex: 1, minWidth: 0, display: 'flex', justifyContent: 'center' }}>
          <div className="set-inner set-inner-wide" style={{ width: '100%', maxWidth: 1280 }}>
            {error && <ResultBanner ok={false}>{error}</ResultBanner>}
            {!cfg ? (
              <div style={{ color: 'var(--text-3)', fontSize: 13 }}>加载中…</div>
            ) : (
              <>
                {tab === 'webhook' && (
                  <WebhookTab cfg={cfg}
                    onChange={updated => { setCfg(updated); showToast('Webhook 配置已保存'); }} />
                )}
                {tab === 'github' && (
                  <GithubTab cfg={cfg} projects={projects}
                    onChange={updated => { setCfg(updated); showToast('GitHub 配置已保存'); }} />
                )}
                {tab === 'scanner' && <ScannerTab projects={projects} />}
                {tab === 'bulk' && <BulkTab projects={projects} />}
              </>
            )}
          </div>
        </div>
      </div>

      <Toast msg={toast.msg} ok={toast.ok} />
    </div>
  );
}
