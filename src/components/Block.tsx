import React, { useState } from 'react';
import Icon from './Icon';
import Markdown from './Markdown';
import { highlightText } from './highlight';
import type { BlockType } from '../data/mock';
import { attachmentDataUrl, decideIssueDraft, openAttachment, writeWorkspaceFile, undoWorkspaceFile } from '../services';

const KW = new Set(['const','let','var','function','return','import','export','from','if','else','for','while','new','await','async','class','def','self','None','True','False','useState','useSearchParams']);

interface Token { c: string | null; t: string }

export function tokenize(code: string): Token[] {
  const tokens: Token[] = [];
  const re = /(\/\/[^\n]*)|("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')|(\b\d+\.?\d*\b)|([A-Za-z_$][\w$]*)|(\s+)|([^\sA-Za-z0-9_$"'\/]+|\/)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(code)) !== null) {
    if (m[1]) tokens.push({ c: 'tok-com', t: m[1] });
    else if (m[2]) tokens.push({ c: 'tok-str', t: m[2] });
    else if (m[3]) tokens.push({ c: 'tok-num', t: m[3] });
    else if (m[4]) {
      const after = code[re.lastIndex];
      if (KW.has(m[4])) tokens.push({ c: 'tok-key', t: m[4] });
      else if (after === '(') tokens.push({ c: 'tok-fn', t: m[4] });
      else tokens.push({ c: null, t: m[4] });
    } else tokens.push({ c: null, t: m[5] || m[6] || '' });
  }
  return tokens;
}

function CodeBlock({ lang, code, projectId, highlight }: { lang: string; code: string; projectId?: string; highlight?: string }) {
  const [copied, setCopied] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState('');
  const [saveErr, setSaveErr] = useState('');
  const tokens = tokenize(code);

  const handleSave = async (subfolder: 'docs' | 'specs' | 'deliverables') => {
    if (!projectId || saving) return;
    const ext = lang || 'txt';
    const ts = new Date().toISOString().slice(0, 16).replace('T', '_').replace(':', '-');
    const prefix = subfolder === 'docs' ? 'doc' : subfolder === 'specs' ? 'spec' : 'deliverable';
    const filename = `${prefix}_${ts}.${ext}`;
    const relPath = `${subfolder}/${filename}`;
    setSaving(true); setSaveErr('');
    try {
      await writeWorkspaceFile(projectId, relPath, code);
      setSaved(relPath);
      setTimeout(() => setSaved(''), 3000);
    } catch (e) { setSaveErr(String(e)); }
    finally { setSaving(false); }
  };

  return (
    <div className="codeblock">
      <div className="codeblock-head">
        <span className="lang">{lang}</span>
        <div style={{ display: 'flex', gap: 4, marginLeft: 'auto', alignItems: 'center' }}>
          {saveErr && <span style={{ fontSize: 'var(--text-micro)', color: 'var(--red)' }}>{saveErr}</span>}
          {saved && <span style={{ fontSize: 'var(--text-micro)', color: 'var(--green)' }}>已存入 {saved}</span>}
          {projectId && (
            <>
              <button className="btn btn-sm" style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }} disabled={saving} onClick={() => handleSave('docs')} title="存入 .autoforge/docs/">
                <Icon name="folder" size={10} />docs
              </button>
              <button className="btn btn-sm" style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }} disabled={saving} onClick={() => handleSave('specs')} title="存入 .autoforge/specs/">
                <Icon name="folder" size={10} />specs
              </button>
              <button className="btn btn-sm" style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }} disabled={saving} onClick={() => handleSave('deliverables')} title="存入 .autoforge/deliverables/">
                <Icon name="folder" size={10} />deliverables
              </button>
            </>
          )}
          <button className="icon-btn" style={{ width: 24, height: 24 }} onClick={() => { setCopied(true); setTimeout(() => setCopied(false), 1200); }}>
            <Icon name={copied ? 'check' : 'copy'} size={13} />
          </button>
        </div>
      </div>
      <pre><code>{tokens.map((tk, i) => tk.c ? <span key={i} className={tk.c}>{highlightText(tk.t, highlight)}</span> : <React.Fragment key={i}>{highlightText(tk.t, highlight)}</React.Fragment>)}</code></pre>
    </div>
  );
}

// ── ArtifactBlock ─────────────────────────────────────────────────────────────

function ArtifactBlock({ b, projectId, highlight, messageId, blockIndex }: { b: Extract<BlockType, { t: 'artifact' }>; projectId?: string; highlight?: string; messageId?: string; blockIndex?: number }) {
  // 需求草稿的决策：优先用持久化在块上的 decided，其次本地乐观状态。
  const [decision, setDecision] = useState<'confirmed' | 'rejected' | ''>(b.decided ?? '');
  const [deciding, setDeciding] = useState<'confirm' | 'reject' | ''>('');
  const [submitErr, setSubmitErr] = useState('');
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState('');
  const [saveErr, setSaveErr] = useState('');

  const meta = b._meta;
  const isDraft = b.kind === 'issue_draft' || b.kind === 'requirement_draft';
  const effectiveProjectId = projectId || meta?.project_id;

  // 在对话 card 内直接确认/拒绝整理好的需求，让整理环节就地闭环。
  const handleDecide = async (decideAs: 'confirm' | 'reject') => {
    if (deciding || decision) return;
    if (!messageId) { setSubmitErr('无法定位消息，请刷新后重试'); return; }
    if (decideAs === 'confirm' && !effectiveProjectId) {
      setSubmitErr('该群聊未绑定项目，无法确认入库');
      return;
    }
    setDeciding(decideAs); setSubmitErr('');
    try {
      await decideIssueDraft({ message_id: messageId, decision: decideAs, block_index: blockIndex });
      setDecision(decideAs === 'confirm' ? 'confirmed' : 'rejected');
    } catch (e) { setSubmitErr(String(e)); }
    finally { setDeciding(''); }
  };

  const handleSaveToWorkspace = async (subfolder: 'docs' | 'specs' | 'deliverables') => {
    if (!effectiveProjectId || saving) return;
    const slug = b.title
      .toLowerCase()
      .replace(/[^\w一-鿿]+/g, '_')
      .replace(/^_+|_+$/g, '')
      .slice(0, 40);
    const relPath = `${subfolder}/${slug || 'artifact'}.md`;
    // Build markdown content from artifact
    const rows = b.rows.map(([k, v]: [string, string]) => `| ${k} | ${v} |`).join('\n');
    const content = `# ${b.title}\n\n${rows ? `| 属性 | 值 |\n|---|---|\n${rows}\n\n` : ''}${b.body}`;
    setSaving(true); setSaveErr('');
    try {
      await writeWorkspaceFile(effectiveProjectId, relPath, content);
      setSaved(relPath);
      setTimeout(() => setSaved(''), 3000);
    } catch (e) { setSaveErr(String(e)); }
    finally { setSaving(false); }
  };

  return (
    <div className={'artifact' + (isDraft ? ' artifact-draft' : '')}>
      <div className="artifact-head">
        <div className="artifact-ic"><Icon name={isDraft ? 'inbox' : 'zap'} size={17} /></div>
        <div style={{ minWidth: 0 }}>
          <div className="artifact-kind">{isDraft ? '需求草稿' : b.kind}</div>
          <div className="artifact-title">{highlightText(b.title, highlight)}</div>
        </div>
      </div>
      <div style={{ padding: '4px 14px' }}>
        {b.rows.map((r, i) => (
          <div className="artifact-row" key={i}>
            <span className="k">{r[0]}</span>
            <span className="v">{r[1]}</span>
          </div>
        ))}
      </div>
      <div className="artifact-body"><Markdown md={b.body} highlight={highlight} /></div>
      <div className="artifact-foot">
        {isDraft ? (
          decision === 'confirmed' ? (
            <span className="chip green" style={{ padding: '3px 10px' }}>
              <Icon name="check" size={12} style={{ marginRight: 4 }} />已确认 · 进入流水线
            </span>
          ) : decision === 'rejected' ? (
            <span className="chip" style={{ padding: '3px 10px', color: 'var(--text-3)' }}>
              <Icon name="x" size={12} style={{ marginRight: 4 }} />已拒绝
            </span>
          ) : (
            <>
              <button className="btn btn-sm btn-primary" disabled={!!deciding} onClick={() => handleDecide('confirm')}>
                <Icon name="check" size={13} />{deciding === 'confirm' ? '确认中…' : '确认需求'}
              </button>
              <button className="btn btn-sm btn-danger" disabled={!!deciding} onClick={() => handleDecide('reject')}>
                <Icon name="x" size={13} />{deciding === 'reject' ? '拒绝中…' : '拒绝需求'}
              </button>
              {submitErr && <span style={{ fontSize: 'var(--text-caption)', color: 'var(--red)' }}>{submitErr}</span>}
            </>
          )
        ) : null}
        {effectiveProjectId && !isDraft && (
          <>
            <button className="btn btn-sm" disabled={saving} onClick={() => handleSaveToWorkspace('docs')} title="存入 .autoforge/docs/">
              <Icon name="folder" size={12} />存入 docs
            </button>
            <button className="btn btn-sm" disabled={saving} onClick={() => handleSaveToWorkspace('specs')} title="存入 .autoforge/specs/">
              <Icon name="folder" size={12} />存入 specs
            </button>
            <button className="btn btn-sm" disabled={saving} onClick={() => handleSaveToWorkspace('deliverables')} title="存入 .autoforge/deliverables/">
              <Icon name="folder" size={12} />存入 deliverables
            </button>
          </>
        )}
        {saveErr && <span style={{ fontSize: 'var(--text-caption)', color: 'var(--red)' }}>{saveErr}</span>}
        {saved && <span style={{ fontSize: 'var(--text-caption)', color: 'var(--green)' }}>已存入 .autoforge/{saved}</span>}
      </div>
    </div>
  );
}

// ── FileWrittenBlock ──────────────────────────────────────────────────────────

function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0 B';
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function FileWrittenBlock({ b, highlight, projectId }: { b: Extract<BlockType, { t: 'file_written' }>; highlight?: string; projectId?: string }) {
  const [expanded, setExpanded] = useState(false);
  const [undone, setUndone] = useState(false);
  const [undoing, setUndoing] = useState(false);

  const undo = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!projectId || undoing || undone) return;
    if (!confirm(`撤销并删除 AI 写入的文件 .autoforge/${b.path}？`)) return;
    setUndoing(true);
    try { await undoWorkspaceFile(projectId, b.path); setUndone(true); }
    catch { /* 失败保持原状，用户可重试 */ }
    finally { setUndoing(false); }
  };

  return (
    <div className="file-written" style={{
      border: `1px solid ${b.error || undone ? 'var(--red)' : 'var(--ember)'}`,
      borderRadius: 10, overflow: 'hidden',
      background: b.error ? 'color-mix(in srgb, var(--red) 8%, transparent)' : undone ? 'var(--bg-3)' : 'var(--ember-tint)',
      fontSize: 'var(--text-control)', opacity: undone ? 0.6 : 1,
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '8px 12px', cursor: 'pointer' }}
        onClick={() => setExpanded(v => !v)}>
        <Icon name={b.error ? 'alert' : 'file'} size={14} style={{ color: b.error ? 'var(--red)' : 'var(--ember)', flexShrink: 0 }} />
        <span style={{ color: 'var(--text-3)', fontSize: 'var(--text-caption)', fontFamily: 'var(--font-mono)', flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', textDecoration: undone ? 'line-through' : 'none' }}>
          .autoforge/{b.path}
        </span>
        <span style={{ fontSize: 'var(--text-caption)', color: b.error ? 'var(--red)' : undone ? 'var(--text-3)' : 'var(--ember)', flexShrink: 0 }}>
          {undone ? '已撤销' : b.error ? '写入失败' : `${formatBytes(b.size_bytes)} 已写入`}
        </span>
        {/* 撤销按钮：仅写入成功、绑定项目、未撤销时可用 */}
        {!b.error && !undone && projectId && (
          <button className="icon-btn" title="撤销此文件写入（删除）" onClick={undo} disabled={undoing} style={{ flexShrink: 0 }}>
            <Icon name="trash" size={13} />
          </button>
        )}
        <Icon name="chevron" size={12} style={{ color: 'var(--text-3)', flexShrink: 0, transform: expanded ? 'rotate(180deg)' : 'none', transition: 'transform .15s' }} />
      </div>
      {expanded && b.preview && (
        <pre style={{ margin: 0, padding: '0 12px 10px', fontSize: 'var(--text-label)', lineHeight: 'var(--leading-relaxed)', color: 'var(--text-2)', whiteSpace: 'pre-wrap', wordBreak: 'break-word', borderTop: '1px solid var(--border)' }}>
          {highlightText(b.preview, highlight)}{b.size_bytes > 200 ? ' …' : ''}
        </pre>
      )}
    </div>
  );
}

// ── Main Block ────────────────────────────────────────────────────────────────

export default function Block({ b, projectId, highlight, messageId, blockIndex }: { b: BlockType; projectId?: string; highlight?: string; messageId?: string; blockIndex?: number }) {
  const [previewUrl, setPreviewUrl] = useState('');
  const [attachmentError, setAttachmentError] = useState('');

  React.useEffect(() => {
    let alive = true;
    setAttachmentError('');
    if (b.t !== 'image' || !b.id) {
      setPreviewUrl('');
      return () => { alive = false; };
    }
    attachmentDataUrl(b.id)
      .then(url => { if (alive) setPreviewUrl(url); })
      .catch(e => { if (alive) setAttachmentError(String(e)); });
    return () => { alive = false; };
  }, [b]);

  const openStoredAttachment = (id?: string) => {
    if (!id) return;
    openAttachment(id).catch(e => setAttachmentError(String(e)));
  };

  if (b.t === 'md') return <Markdown md={b.md} highlight={highlight} />;
  if (b.t === 'code') return <CodeBlock lang={b.lang} code={b.code} projectId={projectId} highlight={highlight} />;
  if (b.t === 'typing') return <div className="typing"><i /><i /><i /></div>;
  if (b.t === 'file') return (
    <div className="att">
      <div className="att-ic" style={{ background: b.color }}><Icon name="file" size={19} /></div>
      <div style={{ minWidth: 0 }}>
        <div className="att-name">{highlightText(b.name, highlight)}</div>
        <div className="att-meta">{highlightText(b.meta, highlight)}</div>
        {attachmentError && <div className="att-error">{attachmentError}</div>}
      </div>
      <button className="icon-btn" style={{ marginLeft: 'auto' }} disabled={!b.id} title="打开附件" onClick={() => openStoredAttachment(b.id)}>
        <Icon name="external" size={16} />
      </button>
    </div>
  );
  if (b.t === 'image') return (
    <div className="att-img">
      {previewUrl
        ? <img src={previewUrl} alt={b.label} />
        : <div className="ph" style={{ background: `linear-gradient(135deg, ${b.color}, ${b.color}99)` }}><Icon name="image" size={30} /></div>}
      <div className="cap">{highlightText(b.label, highlight)}　{highlightText(b.meta, highlight)}</div>
      {attachmentError && <button className="att-img-error" title={attachmentError}><Icon name="alert" size={14} /></button>}
      {b.id && (
        <button className="att-img-open icon-btn" title="打开附件" onClick={() => openStoredAttachment(b.id)}>
          <Icon name="external" size={15} />
        </button>
      )}
    </div>
  );
  if (b.t === 'artifact') return <ArtifactBlock b={b} projectId={projectId} highlight={highlight} messageId={messageId} blockIndex={blockIndex} />;
  if (b.t === 'file_written') return <FileWrittenBlock b={b} highlight={highlight} projectId={projectId} />;
  if (b.t === 'ws_ref') return (
    <div className="att" style={{ borderColor: 'var(--ember)', background: 'var(--ember-tint)' }}>
      <div className="att-ic" style={{ background: 'var(--ember)' }}><Icon name="folder" size={18} /></div>
      <div style={{ minWidth: 0 }}>
        <div className="att-name">{highlightText(b.name, highlight)}</div>
        <div className="att-meta" style={{ fontFamily: 'var(--font-mono)' }}>引用 · .autoforge/{highlightText(b.path, highlight)}</div>
      </div>
    </div>
  );
  return null;
}
