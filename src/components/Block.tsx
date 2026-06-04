import React, { useState } from 'react';
import Icon from './Icon';
import Markdown from './Markdown';
import type { BlockType } from '../data/mock';
import { attachmentDataUrl, openAttachment } from '../services';

const KW = new Set(['const','let','var','function','return','import','export','from','if','else','for','while','new','await','async','class','def','self','None','True','False','useState','useSearchParams']);

interface Token { c: string | null; t: string }

function tokenize(code: string): Token[] {
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

function CodeBlock({ lang, code }: { lang: string; code: string }) {
  const [copied, setCopied] = useState(false);
  const tokens = tokenize(code);
  return (
    <div className="codeblock">
      <div className="codeblock-head">
        <span className="lang">{lang}</span>
        <button className="icon-btn" style={{ width: 24, height: 24 }} onClick={() => { setCopied(true); setTimeout(() => setCopied(false), 1200); }}>
          <Icon name={copied ? 'check' : 'copy'} size={13} />
        </button>
      </div>
      <pre><code>{tokens.map((tk, i) => tk.c ? <span key={i} className={tk.c}>{tk.t}</span> : tk.t)}</code></pre>
    </div>
  );
}

export default function Block({ b }: { b: BlockType }) {
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

  if (b.t === 'md') return <Markdown md={b.md} />;
  if (b.t === 'code') return <CodeBlock lang={b.lang} code={b.code} />;
  if (b.t === 'typing') return <div className="typing"><i /><i /><i /></div>;
  if (b.t === 'file') return (
    <div className="att">
      <div className="att-ic" style={{ background: b.color }}><Icon name="file" size={19} /></div>
      <div style={{ minWidth: 0 }}>
        <div className="att-name">{b.name}</div>
        <div className="att-meta">{b.meta}</div>
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
      <div className="cap">{b.label}　{b.meta}</div>
      {attachmentError && <button className="att-img-error" title={attachmentError}><Icon name="alert" size={14} /></button>}
      {b.id && (
        <button className="att-img-open icon-btn" title="打开附件" onClick={() => openStoredAttachment(b.id)}>
          <Icon name="external" size={15} />
        </button>
      )}
    </div>
  );
  if (b.t === 'artifact') return (
    <div className="artifact">
      <div className="artifact-head">
        <div className="artifact-ic"><Icon name="zap" size={17} /></div>
        <div style={{ minWidth: 0 }}>
          <div className="artifact-kind">{b.kind}</div>
          <div className="artifact-title">{b.title}</div>
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
      <div className="artifact-body">{b.body}</div>
      <div className="artifact-foot">
        <button className="btn btn-sm btn-primary"><Icon name="eye" size={13} />查看详情</button>
        <button className="btn btn-sm">引用</button>
      </div>
    </div>
  );
  return null;
}
