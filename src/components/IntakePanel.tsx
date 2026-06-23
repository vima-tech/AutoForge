import React, { useState, useEffect, useRef } from 'react';
import Icon from './Icon';
import Select from './Select';
import { fmtFull } from '../utils/datetime';
import {
  getIntakeConfig, updateIntakeConfig, syncGithubIssues, bulkImportIssues,
  bulkImportFile, exportBulkTemplate, submitIssue, importIssueAttachment,
  getProjectWebhookToken, regenerateProjectWebhookToken, getWebhookStatus,
  type IntakeConfig, type SyncResult, type BulkResult, type WidgetToken, type WebhookStatus,
} from '../services';
import AttachmentBar, { fileToUpload } from './AttachmentBar';

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
  const [form, setForm] = useState({ title: '', description: '', category: 'Feature', severity: 'medium', repro_steps: '', environment: '', expected: '', actual: '' });
  const [submitting, setSubmitting] = useState(false);
  const [okTitle, setOkTitle] = useState('');
  const [err, setErr] = useState('');
  const [files, setFiles] = useState<File[]>([]);
  const isBug = form.category === 'Bug';

  const submit = async () => {
    if (!form.title.trim()) { setErr('需求标题不能为空'); return; }
    setSubmitting(true); setErr(''); setOkTitle('');
    try {
      const issue = await submitIssue({
        project_id: projectId, title: form.title, description: form.description,
        category: form.category, severity: form.severity,
        ...(isBug ? { repro_steps: form.repro_steps, environment: form.environment, expected: form.expected, actual: form.actual } : {}),
      });
      // 两阶段：需求入库后把附件挂到该需求（图片可供 vision 分析）。
      for (const f of files) {
        try { await importIssueAttachment({ issue_id: issue.id, ...(await fileToUpload(f)) }); }
        catch (e) { console.warn('附件上传失败', f.name, e); }
      }
      setOkTitle(form.title.trim());
      setForm({ title: '', description: '', category: 'Feature', severity: 'medium', repro_steps: '', environment: '', expected: '', actual: '' });
      setFiles([]);
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

        {isBug && (
          <>
            <ISectionLabel>Bug 载体（喂给自主修复的高质量输入）</ISectionLabel>
            <div className="cfg-fields" style={{ gridTemplateColumns: '1fr 1fr', gap: 10, marginBottom: 14 }}>
              <div className="field full" style={{ margin: 0 }}>
                <label>复现步骤</label>
                <textarea rows={2} value={form.repro_steps} onChange={e => setForm(f => ({ ...f, repro_steps: e.target.value }))} placeholder="1. … 2. … 3. …" />
              </div>
              <div className="field" style={{ margin: 0 }}>
                <label>环境</label>
                <input value={form.environment} onChange={e => setForm(f => ({ ...f, environment: e.target.value }))} placeholder="OS / 版本 / 分支" />
              </div>
              <div className="field" style={{ margin: 0 }} />
              <div className="field" style={{ margin: 0 }}>
                <label>期望结果</label>
                <textarea rows={2} value={form.expected} onChange={e => setForm(f => ({ ...f, expected: e.target.value }))} placeholder="应当发生什么" />
              </div>
              <div className="field" style={{ margin: 0 }}>
                <label>实际结果</label>
                <textarea rows={2} value={form.actual} onChange={e => setForm(f => ({ ...f, actual: e.target.value }))} placeholder="实际发生了什么" />
              </div>
            </div>
          </>
        )}

        <ISectionLabel>图片 / 附件</ISectionLabel>
        <div style={{ marginBottom: 14 }}>
          <AttachmentBar staged={files} onStaged={setFiles} />
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
            上次同步：{fmtFull(cfg.github_last_sync)}
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

// File → base64（剥离 data URL 前缀，与 QuickCapture 一致）。
function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const fr = new FileReader();
    fr.onload = () => {
      const s = String(fr.result);
      resolve(s.includes(',') ? s.slice(s.indexOf(',') + 1) : s);
    };
    fr.onerror = () => reject(fr.error);
    fr.readAsDataURL(file);
  });
}

function ProjectBulkTab({ projectId }: { projectId: string }) {
  const [mode, setMode]       = useState<'paste' | 'file'>('paste');
  const [format, setFormat]   = useState<'text' | 'csv' | 'json'>('text');
  const [content, setContent] = useState('');
  const [importing, setImporting] = useState(false);
  const [result, setResult]   = useState<BulkResult | null>(null);
  const [importErr, setImportErr] = useState('');

  const fileInputRef = useRef<HTMLInputElement>(null);
  const [fileImporting, setFileImporting] = useState(false);
  const [fileName, setFileName] = useState('');
  const [tplMsg, setTplMsg] = useState('');

  const lineCount = content.split('\n').filter(l => l.trim()).length;

  const doImport = async () => {
    if (!content.trim()) return;
    setImporting(true); setResult(null); setImportErr('');
    try { setResult(await bulkImportIssues(projectId, format, content)); }
    catch (e) { setImportErr(String(e)); }
    finally { setImporting(false); }
  };

  const doFileImport = async (file: File) => {
    setFileName(file.name);
    setFileImporting(true); setResult(null); setImportErr(''); setTplMsg('');
    try {
      const b64 = await fileToBase64(file);
      setResult(await bulkImportFile(projectId, file.name, b64));
    } catch (e) { setImportErr(String(e)); }
    finally { setFileImporting(false); }
  };

  const doTemplate = async (fmt: 'csv' | 'xlsx') => {
    setTplMsg(''); setImportErr('');
    try {
      const path = await exportBulkTemplate(fmt);
      setTplMsg(`模板已保存到：${path}`);
    } catch (e) { setImportErr(String(e)); }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      <ICard style={{ padding: '16px 18px' }}>
        {/* header + mode switch on one row */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 14 }}>
          <div style={{ width: 32, height: 32, borderRadius: 9, background: 'var(--ember-tint-strong)', display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
            <Icon name="arrowUp" size={15} style={{ color: 'var(--ember)' }} />
          </div>
          <div style={{ minWidth: 0, flex: 1 }}>
            <div style={{ fontWeight: 600, fontSize: 'var(--text-body)' }}>批量导入</div>
            <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', marginTop: 1 }}>一次性入队分析，最多 200 条</div>
          </div>
          <div className="seg" style={{ flexShrink: 0 }}>
            {([['paste', '粘贴文本'], ['file', '上传文件']] as const).map(([id, label]) => (
              <button key={id} type="button" className={mode === id ? 'on' : ''}
                onClick={() => setMode(id)} style={{ padding: '4px 12px' }}>{label}</button>
            ))}
          </div>
        </div>

        {mode === 'paste' ? (
          <>
            {/* compact format chips + inline hint */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 10, flexWrap: 'wrap' }}>
              {(Object.entries(BULK_FORMAT_META) as [keyof typeof BULK_FORMAT_META, { label: string; desc: string }][]).map(([key, meta]) => (
                <button key={key} className={'filter-chip' + (format === key ? ' on' : '')}
                  onClick={() => { setFormat(key); setContent(''); }}>
                  {meta.label}
                </button>
              ))}
              <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', marginLeft: 2 }}>
                {BULK_FORMAT_META[format].desc}
              </span>
            </div>
            <textarea value={content} onChange={e => setContent(e.target.value)}
              placeholder={BULK_PLACEHOLDERS[format]} rows={8}
              style={{ width: '100%', boxSizing: 'border-box', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', lineHeight: 'var(--leading-relaxed)', background: 'var(--bg-3)', border: '1px solid var(--border)', borderRadius: 8, padding: '10px 12px', color: 'var(--text)', resize: 'vertical', outline: 'none' }} />
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginTop: 10 }}>
              <button className="btn btn-primary btn-sm" onClick={doImport} disabled={importing || !content.trim()}>
                <Icon name="arrowUp" size={13} />{importing ? '导入中…' : '开始导入'}
              </button>
              <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>{lineCount} 行</span>
            </div>
          </>
        ) : (
          <>
            {/* upload dropzone */}
            <input ref={fileInputRef} type="file" accept=".csv,.xlsx,.xls,.ods" style={{ display: 'none' }}
              onChange={e => { const f = e.target.files?.[0]; if (f) doFileImport(f); e.target.value = ''; }} />
            <button onClick={() => fileInputRef.current?.click()} disabled={fileImporting}
              style={{ width: '100%', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: 6, padding: '22px 16px', background: 'var(--bg-3)', border: '1px dashed var(--border-strong)', borderRadius: 10, cursor: fileImporting ? 'default' : 'pointer', color: 'var(--text-2)' }}>
              <Icon name="upload" size={20} style={{ color: 'var(--ember)', ...(fileImporting ? { animation: 'spin 1s linear infinite' } : {}) }} />
              <span style={{ fontSize: 'var(--text-control)', fontWeight: 600, color: 'var(--text)' }}>
                {fileImporting ? '解析导入中…' : (fileName || '选择文件导入')}
              </span>
              <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>支持 .csv / .xlsx / .xls，首行表头 title · description · category · severity</span>
            </button>
            {/* template links */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 10, fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>
              <span>需要模板？</span>
              <button className="btn btn-ghost btn-sm" onClick={() => doTemplate('csv')} style={{ padding: '3px 9px' }}>
                <Icon name="download" size={12} />CSV
              </button>
              <button className="btn btn-ghost btn-sm" onClick={() => doTemplate('xlsx')} style={{ padding: '3px 9px' }}>
                <Icon name="download" size={12} />Excel
              </button>
            </div>
            {tplMsg && (
              <div style={{ marginTop: 8, fontSize: 'var(--text-caption)', color: 'var(--text-2)', wordBreak: 'break-all' }}>{tplMsg}</div>
            )}
          </>
        )}
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
    </div>
  );
}

// ── ProjectWebhookTab ─────────────────────────────────────────────────────────
// 本项目专属的 HTTP Webhook 接入凭证。token 决定需求落到哪个项目（不再信任 payload
// 里的 project_id），可独立轮换/吊销。全局开关在「设置 → Webhook 集成」。

function ProjectWebhookTab({ projectId, cfg }: { projectId: string; cfg: IntakeConfig }) {
  const [token, setToken]   = useState<WidgetToken | null>(null);
  const [status, setStatus] = useState<WebhookStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [err, setErr]       = useState('');
  const [copied, setCopied] = useState<'token' | 'curl' | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [rotating, setRotating]     = useState(false);

  useEffect(() => {
    setLoading(true); setErr('');
    Promise.all([getProjectWebhookToken(projectId), getWebhookStatus().catch(() => null)])
      .then(([t, s]) => { setToken(t); setStatus(s); })
      .catch(e => setErr(String(e)))
      .finally(() => setLoading(false));
  }, [projectId]);

  const rotate = async () => {
    setRotating(true);
    try {
      const t = await regenerateProjectWebhookToken(projectId);
      setToken(t); setConfirming(false);
    } catch (e) { setErr(String(e)); }
    finally { setRotating(false); }
  };

  const copy = async (text: string, which: 'token' | 'curl') => {
    await navigator.clipboard.writeText(text).catch(() => {});
    setCopied(which); setTimeout(() => setCopied(null), 1200);
  };

  if (loading) return <div style={{ color: 'var(--text-3)', fontSize: 'var(--text-control)' }}>加载中…</div>;
  if (err) return <IResultBanner ok={false}>{err}</IResultBanner>;

  const tok = token?.token ?? '';
  const curlExample = `curl -X POST http://127.0.0.1:${cfg.webhook_port}/webhook/issues \\
  -H "Authorization: Bearer ${tok || '<token>'}" \\
  -H "Content-Type: application/json" \\
  -d '{"title":"需求标题","description":"详细描述"}'`;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <ICard>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 18 }}>
          <div style={{ width: 34, height: 34, borderRadius: 9, background: 'rgba(232,119,46,.15)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Icon name="zap" size={16} style={{ color: 'var(--ember)' }} />
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ fontWeight: 600, fontSize: 'var(--text-body)' }}>Webhook 接入凭证</div>
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 1 }}>本项目专属 token，命中即自动进需求分析；外部系统用它作 Bearer 推送需求</div>
          </div>
          <span className={'chip ' + (status?.running ? 'green' : '')} style={{ padding: '3px 10px', fontSize: 'var(--text-caption)' }}>
            <span style={{ width: 6, height: 6, borderRadius: '50%', background: status?.running ? 'var(--green)' : 'var(--text-3)', display: 'inline-block', marginRight: 5 }} />
            {status?.running ? `服务运行中 :${status.port}` : '服务未启用'}
          </span>
        </div>

        {!status?.running && (
          <div style={{ background: 'rgba(212,160,90,.1)', border: '1px solid rgba(212,160,90,.3)', borderRadius: 10, padding: '10px 14px', fontSize: 'var(--text-label)', color: 'var(--text-2)', display: 'flex', gap: 8, marginBottom: 14 }}>
            <Icon name="alert" size={13} style={{ flexShrink: 0, marginTop: 1, color: 'var(--amber)' }} />
            <div>Webhook 服务当前未运行，token 暂不可用。请到「设置 → Webhook 集成」启用 Webhook。</div>
          </div>
        )}

        <ISectionLabel>本项目 Token</ISectionLabel>
        <div style={{ display: 'flex', gap: 6, marginBottom: 14 }}>
          <input readOnly value={tok}
            style={{ flex: 1, fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 9, padding: '8px 12px', color: 'var(--text)' }} />
          <button className="btn btn-sm" onClick={() => copy(tok, 'token')} style={{ flexShrink: 0 }}>
            <Icon name={copied === 'token' ? 'check' : 'copy'} size={13} />复制
          </button>
        </div>

        {!confirming ? (
          <button className="btn btn-sm" onClick={() => setConfirming(true)}>
            <Icon name="refresh" size={13} />轮换 Token
          </button>
        ) : (
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)' }}>旧 token 将立即失效，确认轮换？</span>
            <button className="btn btn-danger btn-sm" onClick={rotate} disabled={rotating}>
              <Icon name="refresh" size={13} />{rotating ? '轮换中…' : '确认轮换'}
            </button>
            <button className="btn btn-ghost btn-sm" onClick={() => setConfirming(false)} disabled={rotating}>取消</button>
          </div>
        )}
      </ICard>

      <ICard>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 10 }}>
          <ISectionLabel>curl 示例</ISectionLabel>
          <button className="icon-btn" style={{ width: 26, height: 26 }} title="复制" onClick={() => copy(curlExample, 'curl')}>
            <Icon name={copied === 'curl' ? 'check' : 'copy'} size={13} />
          </button>
        </div>
        <pre style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', lineHeight: 'var(--leading-relaxed)', color: 'var(--text-2)', background: 'var(--bg-3)', borderRadius: 8, padding: '12px 14px', overflowX: 'auto', margin: 0, whiteSpace: 'pre-wrap', wordBreak: 'break-all', border: '1px solid var(--border)' }}>
          {curlExample}
        </pre>
        <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 10 }}>
          项目由 token 决定，<code style={{ fontFamily: 'var(--font-mono)' }}>project_id</code> 可省略；若填写则必须与本项目一致，否则被拒。
        </div>
      </ICard>
    </div>
  );
}

// ── IntakePanel ───────────────────────────────────────────────────────────────

type IntakeSubTab = 'manual' | 'github' | 'webhook' | 'bulk';

const INTAKE_SUB_TABS: { id: IntakeSubTab; label: string; ic: string }[] = [
  { id: 'manual',  label: '手动提交', ic: 'send' },
  { id: 'github',  label: 'GitHub', ic: 'code' },
  { id: 'webhook', label: 'Webhook', ic: 'zap' },
  { id: 'bulk',    label: '批量导入', ic: 'arrowUp' },
];

export default function IntakePanel({ projectId, tabOrder }: { projectId: string; tabOrder?: IntakeSubTab[] }) {
  // 各页面可自定义 tab 顺序；缺省走 INTAKE_SUB_TABS 原序。未知 id 忽略，缺失的 tab 补齐到末尾。
  const tabs = React.useMemo(() => {
    if (!tabOrder || tabOrder.length === 0) return INTAKE_SUB_TABS;
    const byId = new Map(INTAKE_SUB_TABS.map(t => [t.id, t]));
    const ordered = tabOrder.map(id => byId.get(id)).filter(Boolean) as typeof INTAKE_SUB_TABS;
    const rest = INTAKE_SUB_TABS.filter(t => !tabOrder.includes(t.id));
    return [...ordered, ...rest];
  }, [tabOrder]);

  const [subTab, setSubTab] = useState<IntakeSubTab>(tabs[0].id);
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
        {tabs.map(t => (
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
            {subTab === 'webhook' && <ProjectWebhookTab projectId={projectId} cfg={cfg} />}
            {subTab === 'bulk'    && <ProjectBulkTab projectId={projectId} />}
          </>
        ) : null)}
      </div>
    </div>
  );
}
