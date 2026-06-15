import React, { useState, useEffect } from 'react';
import Icon from './Icon';
import Select from './Select';
import {
  getIntakeConfig, updateIntakeConfig, syncGithubIssues, runCodeScan, bulkImportIssues, submitIssue,
  type IntakeConfig, type SyncResult, type ScanResult, type BulkResult,
} from '../services';

// ── Intake helpers ────────────────────────────────────────────────────────────

function ICard({ children, style }: { children: React.ReactNode; style?: React.CSSProperties }) {
  return (
    <div style={{ background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 14, padding: '18px 20px', ...style }}>
      {children}
    </div>
  );
}

function ISectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ fontSize: 'var(--text-caption)', fontWeight: 700, letterSpacing: '.07em', textTransform: 'uppercase', color: 'var(--text-faint)', marginBottom: 10 }}>
      {children}
    </div>
  );
}

function IResultBanner({ ok, children }: { ok?: boolean; children: React.ReactNode }) {
  return (
    <div style={{
      background: ok === false ? 'rgba(219,90,64,.1)' : 'rgba(79,157,107,.1)',
      border: `1px solid ${ok === false ? 'rgba(219,90,64,.3)' : 'rgba(79,157,107,.3)'}`,
      borderRadius: 10, padding: '10px 14px', fontSize: 'var(--text-control)',
      color: ok === false ? 'var(--red)' : 'var(--green)',
      display: 'flex', alignItems: 'flex-start', gap: 8,
    }}>
      <Icon name={ok === false ? 'alert' : 'check'} size={14} style={{ flexShrink: 0, marginTop: 1 }} />
      <div>{children}</div>
    </div>
  );
}

// ── ProjectManualTab ──────────────────────────────────────────────────────────

function ProjectManualTab({ projectId }: { projectId: string }) {
  const [form, setForm] = useState({ title: '', description: '', category: 'Feature', severity: 'medium' });
  const [submitting, setSubmitting] = useState(false);
  const [okTitle, setOkTitle] = useState('');
  const [err, setErr] = useState('');

  const submit = async () => {
    if (!form.title.trim()) { setErr('需求标题不能为空'); return; }
    setSubmitting(true); setErr(''); setOkTitle('');
    try {
      await submitIssue({ project_id: projectId, ...form });
      setOkTitle(form.title.trim());
      setForm({ title: '', description: '', category: 'Feature', severity: 'medium' });
    } catch (e) { setErr(String(e)); }
    finally { setSubmitting(false); }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <ICard>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 18 }}>
          <div style={{ width: 34, height: 34, borderRadius: 9, background: 'var(--ember-tint)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Icon name="send" size={16} style={{ color: 'var(--ember)' }} />
          </div>
          <div>
            <div style={{ fontWeight: 600, fontSize: 'var(--text-body)' }}>手动提交</div>
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 1 }}>单条录入需求，立即进入分析队列</div>
          </div>
        </div>

        <ISectionLabel>需求内容</ISectionLabel>
        <div className="cfg-fields" style={{ gridTemplateColumns: '1fr 1fr', gap: 10, marginBottom: 14 }}>
          <div className="field full" style={{ margin: 0 }}>
            <label>需求标题</label>
            <input value={form.title} onChange={e => setForm(f => ({ ...f, title: e.target.value }))} placeholder="简洁描述需求" />
          </div>
          <div className="field full" style={{ margin: 0 }}>
            <label>详细描述</label>
            <textarea rows={3} value={form.description} onChange={e => setForm(f => ({ ...f, description: e.target.value }))} placeholder="背景、期望行为、截图说明等" />
          </div>
          <div className="field" style={{ margin: 0 }}>
            <label>分类</label>
            <Select value={form.category} onChange={val => setForm(f => ({ ...f, category: val }))}
              options={['Feature', 'Bug', 'Improvement', 'Debt'].map(v => ({ value: v, label: v }))} />
          </div>
          <div className="field" style={{ margin: 0 }}>
            <label>严重级别</label>
            <Select value={form.severity} onChange={val => setForm(f => ({ ...f, severity: val }))}
              options={[{ value: 'critical', label: 'Critical' }, { value: 'high', label: 'High' }, { value: 'medium', label: 'Medium' }, { value: 'low', label: 'Low' }]} />
          </div>
        </div>

        {okTitle && <IResultBanner ok>已提交「<strong>{okTitle}</strong>」，需求已进入分析队列</IResultBanner>}
        {err && <IResultBanner ok={false}>{err}</IResultBanner>}
        <button className="btn btn-primary" style={{ marginTop: okTitle || err ? 10 : 0, alignSelf: 'flex-start' }}
          onClick={submit} disabled={submitting || !form.title.trim()}>
          <Icon name="send" size={14} />{submitting ? '提交中…' : '提交需求'}
        </button>
      </ICard>
    </div>
  );
}

// ── ProjectGithubTab ──────────────────────────────────────────────────────────

function ProjectGithubTab({ projectId, cfg, onCfgChange }: {
  projectId: string;
  cfg: IntakeConfig;
  onCfgChange: (c: IntakeConfig) => void;
}) {
  const [form, setForm] = useState({ owner: cfg.github_owner, repo: cfg.github_repo, token: cfg.github_token });
  const [saving, setSaving]       = useState(false);
  const [syncing, setSyncing]     = useState(false);
  const [syncResult, setSyncResult] = useState<SyncResult | null>(null);
  const [syncErr, setSyncErr]     = useState('');
  const [saveOk, setSaveOk]       = useState<boolean | null>(null);

  const save = async () => {
    setSaving(true); setSaveOk(null);
    try {
      const updated = await updateIntakeConfig({
        github_owner: form.owner.trim(),
        github_repo: form.repo.trim(),
        github_token: form.token.trim(),
        github_project_id: projectId,
      });
      onCfgChange(updated);
      setSaveOk(true);
      setTimeout(() => setSaveOk(null), 2500);
    } catch { setSaveOk(false); }
    finally { setSaving(false); }
  };

  const sync = async () => {
    setSyncing(true); setSyncResult(null); setSyncErr('');
    try {
      await updateIntakeConfig({ github_owner: form.owner.trim(), github_repo: form.repo.trim(), github_token: form.token.trim(), github_project_id: projectId });
      const r = await syncGithubIssues();
      setSyncResult(r);
    } catch (e) { setSyncErr(String(e)); }
    finally { setSyncing(false); }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <ICard>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 18 }}>
          <div style={{ width: 34, height: 34, borderRadius: 9, background: 'rgba(79,142,209,.15)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Icon name="code" size={16} style={{ color: 'var(--blue)' }} />
          </div>
          <div>
            <div style={{ fontWeight: 600, fontSize: 'var(--text-body)' }}>GitHub Issues 同步</div>
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 1 }}>单向拉取仓库 Issues，自动去重导入本项目</div>
          </div>
        </div>

        <ISectionLabel>仓库信息</ISectionLabel>
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

        <ISectionLabel>认证</ISectionLabel>
        <div className="field" style={{ margin: '0 0 14px' }}>
          <label>GitHub Token（私有仓库必填，公开仓库可留空）</label>
          <input type="password" value={form.token} onChange={e => setForm(f => ({ ...f, token: e.target.value }))}
            placeholder="ghp_xxxxxxxxxxxx" style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)' }} />
        </div>

        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <button className="btn btn-primary" onClick={save} disabled={saving}>
            <Icon name="check" size={14} />{saving ? '保存中…' : '保存配置'}
          </button>
          {saveOk === true && <span style={{ fontSize: 'var(--text-label)', color: 'var(--green)' }}>✓ 已保存</span>}
          {saveOk === false && <span style={{ fontSize: 'var(--text-label)', color: 'var(--red)' }}>保存失败</span>}
        </div>
      </ICard>

      <ICard>
        <ISectionLabel>立即同步</ISectionLabel>
        {cfg.github_last_sync && (
          <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginBottom: 10 }}>
            上次同步：{new Date(cfg.github_last_sync).toLocaleString('zh-CN')}
          </div>
        )}
        {syncResult && (
          <IResultBanner ok>
            同步完成：导入 <strong>{syncResult.imported}</strong> 条，
            跳过重复 {syncResult.skipped} 条
            {syncResult.errors > 0 && `，错误 ${syncResult.errors} 条`}
          </IResultBanner>
        )}
        {syncErr && <IResultBanner ok={false}>{syncErr}</IResultBanner>}
        <button className="btn" style={{ marginTop: syncResult || syncErr ? 10 : 0 }}
          onClick={sync} disabled={syncing || !form.owner || !form.repo}>
          <Icon name="refresh" size={14} style={{ animation: syncing ? 'spin 1s linear infinite' : undefined }} />
          {syncing ? '同步中…' : '立即同步'}
        </button>
      </ICard>
    </div>
  );
}

// ── ProjectScannerTab ─────────────────────────────────────────────────────────

const SCAN_ITEMS_DEF = [
  { key: 'todo' as const,  icon: 'code',   color: '#8b7ad8', bg: 'rgba(139,122,216,.12)', title: 'TODO / FIXME 注释', desc: '扫描代码中的 TODO、FIXME、HACK、XXX 注释并作为 Debt 类需求入队' },
  { key: 'cargo' as const, icon: 'shield', color: '#db5a40', bg: 'rgba(219,90,64,.12)',   title: 'cargo audit',       desc: '检测 Rust 依赖中的已知安全漏洞（需 Cargo.lock）' },
  { key: 'npm' as const,   icon: 'box',    color: '#4f9d6b', bg: 'rgba(79,157,107,.12)',  title: 'npm audit',         desc: '检测 Node.js 依赖中的安全漏洞（需 package-lock.json）' },
];

function ProjectScannerTab({ projectId }: { projectId: string }) {
  const [scanTypes, setScanTypes] = useState({ todo: true, cargo: true, npm: true });
  const [scanning, setScanning]   = useState(false);
  const [result, setResult]       = useState<ScanResult | null>(null);
  const [scanErr, setScanErr]     = useState('');

  const scan = async () => {
    setScanning(true); setResult(null); setScanErr('');
    const types: string[] = [];
    if (scanTypes.todo) types.push('todo');
    if (scanTypes.cargo) types.push('cargo_audit');
    if (scanTypes.npm) types.push('npm_audit');
    try { setResult(await runCodeScan(projectId, types)); }
    catch (e) { setScanErr(String(e)); }
    finally { setScanning(false); }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <ICard>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 16 }}>
          <div style={{ width: 34, height: 34, borderRadius: 9, background: 'rgba(79,157,107,.15)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Icon name="search" size={16} style={{ color: 'var(--green)' }} />
          </div>
          <div>
            <div style={{ fontWeight: 600, fontSize: 'var(--text-body)' }}>代码扫描</div>
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 1 }}>主动发现代码库中的问题并自动入队</div>
          </div>
        </div>

        <ISectionLabel>扫描类型</ISectionLabel>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8, marginBottom: 14 }}>
          {SCAN_ITEMS_DEF.map(item => (
            <div key={item.key} onClick={() => setScanTypes(s => ({ ...s, [item.key]: !s[item.key] }))}
              style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '12px 14px', borderRadius: 10, cursor: 'pointer', background: scanTypes[item.key] ? item.bg : 'var(--bg-3)', border: `1px solid ${scanTypes[item.key] ? item.color + '44' : 'var(--border)'}`, transition: 'background .15s, border-color .15s' }}>
              <div style={{ width: 30, height: 30, borderRadius: 8, background: item.bg, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
                <Icon name={item.icon as any} size={14} style={{ color: item.color }} />
              </div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontWeight: 500, fontSize: 'var(--text-control)' }}>{item.title}</div>
                <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 2 }}>{item.desc}</div>
              </div>
              <div style={{ width: 18, height: 18, borderRadius: 5, flexShrink: 0, background: scanTypes[item.key] ? item.color : 'transparent', border: `2px solid ${scanTypes[item.key] ? item.color : 'var(--border-strong)'}`, display: 'flex', alignItems: 'center', justifyContent: 'center', transition: 'background .15s, border-color .15s' }}>
                {scanTypes[item.key] && <Icon name="check" size={10} style={{ color: '#fff' }} />}
              </div>
            </div>
          ))}
        </div>

        {result && <IResultBanner ok>扫描完成：发现 <strong>{result.found}</strong> 处，新建 Issue <strong>{result.new_issues}</strong> 条</IResultBanner>}
        {scanErr && <IResultBanner ok={false}>{scanErr}</IResultBanner>}
        <button className="btn btn-primary" style={{ marginTop: result || scanErr ? 10 : 0, alignSelf: 'flex-start' }}
          onClick={scan} disabled={scanning}>
          <Icon name="search" size={14} style={{ animation: scanning ? 'spin 1s linear infinite' : undefined }} />
          {scanning ? '扫描中…' : '运行扫描'}
        </button>
      </ICard>
    </div>
  );
}

// ── ProjectBulkTab ────────────────────────────────────────────────────────────

const BULK_FORMAT_META = {
  text: { label: '纯文本', desc: '每行一个需求标题' },
  csv:  { label: 'CSV',   desc: '支持 title / description / category / severity 列' },
  json: { label: 'JSON',  desc: '对象数组，字段同 CSV' },
} as const;

const BULK_PLACEHOLDERS: Record<string, string> = {
  text: '每行一个需求标题，例如：\n添加用户头像上传功能\n优化搜索响应速度\n修复登录超时 Bug',
  csv:  'title,description,category,severity\n添加头像上传,用户可上传个人头像,Feature,medium\n修复登录 Bug,,Bug,high',
  json: '[\n  { "title": "需求1", "description": "描述", "category": "Feature", "severity": "medium" },\n  { "title": "需求2" }\n]',
};

function ProjectBulkTab({ projectId }: { projectId: string }) {
  const [format, setFormat]   = useState<'text' | 'csv' | 'json'>('text');
  const [content, setContent] = useState('');
  const [importing, setImporting] = useState(false);
  const [result, setResult]   = useState<BulkResult | null>(null);
  const [importErr, setImportErr] = useState('');

  const doImport = async () => {
    if (!content.trim()) return;
    setImporting(true); setResult(null); setImportErr('');
    try { setResult(await bulkImportIssues(projectId, format, content)); }
    catch (e) { setImportErr(String(e)); }
    finally { setImporting(false); }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <ICard>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 18 }}>
          <div style={{ width: 34, height: 34, borderRadius: 9, background: 'rgba(224,163,46,.15)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Icon name="arrowUp" size={16} style={{ color: 'var(--amber)' }} />
          </div>
          <div>
            <div style={{ fontWeight: 600, fontSize: 'var(--text-body)' }}>批量导入</div>
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 1 }}>粘贴多条需求，一次性入队分析（最多 200 条）</div>
          </div>
        </div>

        <ISectionLabel>格式</ISectionLabel>
        <div style={{ display: 'flex', gap: 6, marginBottom: 14 }}>
          {(Object.entries(BULK_FORMAT_META) as [keyof typeof BULK_FORMAT_META, { label: string; desc: string }][]).map(([key, meta]) => (
            <button key={key} className={'btn btn-sm' + (format === key ? ' btn-primary' : '')}
              onClick={() => { setFormat(key); setContent(''); }}
              style={{ flexDirection: 'column', alignItems: 'flex-start', padding: '8px 12px', height: 'auto', gap: 2 }}>
              <span style={{ fontWeight: 600, fontSize: 'var(--text-label)', color: format === key ? 'var(--bubble-me-text)' : 'var(--text)' }}>{meta.label}</span>
              <span style={{ fontSize: 'var(--text-caption)', color: format === key ? 'rgba(255,255,255,.75)' : 'var(--text-2)', fontWeight: 400 }}>{meta.desc}</span>
            </button>
          ))}
        </div>
      </ICard>

      <ICard>
        <ISectionLabel>内容（{content.split('\n').filter(l => l.trim()).length} 行）</ISectionLabel>
        <textarea value={content} onChange={e => setContent(e.target.value)}
          placeholder={BULK_PLACEHOLDERS[format]} rows={10}
          style={{ width: '100%', boxSizing: 'border-box', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', lineHeight: 'var(--leading-relaxed)', background: 'var(--bg-3)', border: '1px solid var(--border)', borderRadius: 8, padding: '10px 12px', color: 'var(--text)', resize: 'vertical', outline: 'none' }} />
      </ICard>

      {result && (
        <IResultBanner ok={result.errors.length === 0}>
          <div>导入完成：共 {result.total} 条，成功 <strong>{result.imported}</strong>，跳过 {result.skipped}</div>
          {result.errors.length > 0 && (
            <div style={{ marginTop: 6 }}>
              {result.errors.slice(0, 5).map((e, i) => <div key={i} style={{ marginTop: 2 }}>• {e}</div>)}
              {result.errors.length > 5 && <div>…及 {result.errors.length - 5} 条其他错误</div>}
            </div>
          )}
        </IResultBanner>
      )}
      {importErr && <IResultBanner ok={false}>{importErr}</IResultBanner>}

      <button className="btn btn-primary" style={{ alignSelf: 'flex-start' }}
        onClick={doImport} disabled={importing || !content.trim()}>
        <Icon name="arrowUp" size={14} />{importing ? '导入中…' : '开始导入'}
      </button>
    </div>
  );
}

// ── IntakePanel ───────────────────────────────────────────────────────────────

type IntakeSubTab = 'manual' | 'github' | 'scanner' | 'bulk';

const INTAKE_SUB_TABS: { id: IntakeSubTab; label: string; ic: string }[] = [
  { id: 'manual',  label: '手动提交', ic: 'send' },
  { id: 'github',  label: 'GitHub', ic: 'code' },
  { id: 'scanner', label: '代码扫描', ic: 'search' },
  { id: 'bulk',    label: '批量导入', ic: 'arrowUp' },
];

export default function IntakePanel({ projectId }: { projectId: string }) {
  const [subTab, setSubTab] = useState<IntakeSubTab>('manual');
  const [cfg, setCfg]       = useState<IntakeConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadErr, setLoadErr] = useState('');

  useEffect(() => {
    setLoading(true);
    getIntakeConfig()
      .then(setCfg)
      .catch(e => setLoadErr(String(e)))
      .finally(() => setLoading(false));
  }, [projectId]);

  return (
    <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden', minHeight: 0 }}>
      {/* sub-tab bar */}
      <div style={{ display: 'flex', gap: 2, padding: '10px 24px 0', borderBottom: '1px solid var(--border)', flexShrink: 0 }}>
        {INTAKE_SUB_TABS.map(t => (
          <button key={t.id} onClick={() => setSubTab(t.id)}
            style={{ background: 'none', border: 'none', padding: '6px 14px', cursor: 'pointer', fontSize: 'var(--text-control)', fontWeight: 600, display: 'flex', alignItems: 'center', gap: 6, color: subTab === t.id ? 'var(--ember)' : 'var(--text-3)', borderBottom: subTab === t.id ? '2px solid var(--ember)' : '2px solid transparent', marginBottom: -1, transition: 'color .15s' }}>
            <Icon name={t.ic as any} size={13} />{t.label}
          </button>
        ))}
      </div>

      {/* content */}
      <div style={{ flex: 1, overflowY: 'auto', padding: '20px 24px' }}>
        {/* Manual submit only needs the project id, so it renders regardless of intake-config load. */}
        {subTab === 'manual' && <ProjectManualTab projectId={projectId} />}
        {subTab !== 'manual' && loadErr && <IResultBanner ok={false}>{loadErr}</IResultBanner>}
        {subTab !== 'manual' && (loading ? (
          <div style={{ color: 'var(--text-3)', fontSize: 'var(--text-control)' }}>加载中…</div>
        ) : cfg ? (
          <>
            {subTab === 'github'  && <ProjectGithubTab projectId={projectId} cfg={cfg} onCfgChange={setCfg} />}
            {subTab === 'scanner' && <ProjectScannerTab projectId={projectId} />}
            {subTab === 'bulk'    && <ProjectBulkTab projectId={projectId} />}
          </>
        ) : null)}
      </div>
    </div>
  );
}
