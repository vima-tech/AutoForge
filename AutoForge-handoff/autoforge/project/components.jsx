/* ============================================================
   AutoForge — shared components (icons, markdown, bubbles)
   ============================================================ */
const { useState, useRef, useEffect } = React;

/* ---------------- Icon set (stroke, 24 viewbox) ---------------- */
const PATHS = {
  home: 'M3 11.5 12 4l9 7.5M5 10v10h5v-6h4v6h5V10',
  chat: 'M21 15a2 2 0 0 1-2 2H8l-4 4V5a2 2 0 0 1 2-2h13a2 2 0 0 1 2 2z',
  audit: 'M9 11l3 3 8-8M21 12v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h9',
  settings: 'M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z',
  search: 'M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.3-4.3',
  plus: 'M12 5v14M5 12h14',
  send: 'M22 2 11 13M22 2l-7 20-4-9-9-4z',
  paperclip: 'M21.4 11.05 12.2 20.3a5 5 0 0 1-7.07-7.07l9.19-9.19a3.33 3.33 0 0 1 4.71 4.71l-9.2 9.19a1.67 1.67 0 0 1-2.36-2.36l8.49-8.48',
  image: 'M3 5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2zM8.5 11a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3zM21 15l-5-5L5 21',
  at: 'M12 16a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM16 8v5a3 3 0 0 0 6 0v-1a10 10 0 1 0-3.92 7.94',
  file: 'M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8zM14 2v6h6',
  box: 'M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16zM3.27 6.96 12 12l8.73-5.04M12 22V12',
  inbox: 'M22 12h-6l-2 3h-4l-2-3H2M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z',
  cpu: 'M4 4h16v16H4zM9 9h6v6H9zM9 1v3M15 1v3M9 20v3M15 20v3M20 9h3M20 14h3M1 9h3M1 14h3',
  clock: 'M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 6v6l4 2',
  check: 'M20 6 9 17l-5-5',
  code: 'M16 18l6-6-6-6M8 6l-6 6 6 6',
  eye: 'M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7zM12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z',
  merge: 'M6 3v12M18 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM6 15a9 9 0 0 0 9-9h0a3 3 0 0 0 3 3',
  zap: 'M13 2 3 14h9l-1 8 10-12h-9l1-8z',
  bot: 'M12 8V4H8M4 8h16a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2zM9 13v2M15 13v2',
  brain: 'M9.5 2A2.5 2.5 0 0 1 12 4.5v15a2.5 2.5 0 0 1-4.96.44 2.5 2.5 0 0 1-2.96-3.08 3 3 0 0 1-.34-5.58 2.5 2.5 0 0 1 1.32-4.24 2.5 2.5 0 0 1 1.98-3A2.5 2.5 0 0 1 9.5 2zM14.5 2A2.5 2.5 0 0 0 12 4.5',
  shield: 'M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z',
  sun: 'M12 17a5 5 0 1 0 0-10 5 5 0 0 0 0 10zM12 1v2M12 21v2M4.2 4.2l1.4 1.4M18.4 18.4l1.4 1.4M1 12h2M21 12h2M4.2 19.8l1.4-1.4M18.4 5.6l1.4-1.4',
  moon: 'M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z',
  bell: 'M18 8a6 6 0 1 0-12 0c0 7-3 9-3 9h18s-3-2-3-9M13.73 21a2 2 0 0 1-3.46 0',
  chevDown: 'M6 9l6 6 6-6',
  chevRight: 'M9 18l6-6-6-6',
  external: 'M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6M15 3h6v6M10 14 21 3',
  refresh: 'M23 4v6h-6M1 20v-6h6M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15',
  x: 'M18 6 6 18M6 6l12 12',
  play: 'M5 3l14 9-14 9z',
  pause: 'M6 4h4v16H6zM14 4h4v16h-4z',
  square: 'M5 5h14v14H5z',
  alert: 'M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0zM12 9v4M12 17h.01',
  copy: 'M9 9h11a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H9a2 2 0 0 1-2-2v-9a2 2 0 0 1 2-2zM5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1',
  flask: 'M9 2h6M10 2v6.5L4.5 18a2 2 0 0 0 1.8 3h11.4a2 2 0 0 0 1.8-3L14 8.5V2',
  key: 'M21 2l-2 2m-7.6 7.6a5 5 0 1 1-7.1 7.1 5 5 0 0 1 7.1-7.1zm0 0L15 8m0 0l3 3m-3-3 2.7-2.7',
  sliders: 'M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6',
  layers: 'M12 2 2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5',
  trash: 'M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2',
  arrowUp: 'M12 19V5M5 12l7-7 7 7',
  arrowDown: 'M12 5v14M19 12l-7 7-7-7',
  grid: 'M3 3h7v7H3zM14 3h7v7h-7zM14 14h7v7h-7zM3 14h7v7H3z',
  columns: 'M3 3h18v18H3zM12 3v18',
  rows: 'M3 3h18v18H3zM3 12h18',
  dollar: 'M12 1v22M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6',
};
function Icon({ name, size, style, className }) {
  const d = PATHS[name];
  return React.createElement('svg', { viewBox: '0 0 24 24', width: size, height: size, fill: 'none', stroke: 'currentColor', strokeWidth: 2, strokeLinecap: 'round', strokeLinejoin: 'round', style, className },
    (d || '').split('M').filter(Boolean).map((seg, i) => React.createElement('path', { key: i, d: 'M' + seg })));
}

/* ---------------- Avatar ---------------- */
function Avatar({ agent, size = 40, status, square }) {
  const a = typeof agent === 'string' ? AF.A[agent] : agent;
  const bg = a ? a.color : '#e8772e';
  const txt = a ? a.initial : '?';
  return (
    <div className="av" style={{ width: size, height: size, background: bg, fontSize: size * 0.4, borderRadius: square ? size * 0.3 : size * 0.32 }}>
      {txt}
      {status && <span className="av-status" style={{ background: status === 'online' ? 'var(--green)' : status === 'busy' ? 'var(--amber)' : 'var(--text-faint)' }} />}
    </div>
  );
}

function MeAvatar({ size = 40 }) {
  return <div className="av" style={{ width: size, height: size, fontSize: size * 0.4, borderRadius: size * 0.32, background: 'linear-gradient(135deg,#3a2c1c,#211a13)', color: 'var(--ember-soft)', border: '1px solid var(--border-strong)' }}>管</div>;
}

/* ---------------- tiny markdown renderer ---------------- */
function renderInline(text) {
  // escape
  let s = text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  s = s.replace(/`([^`]+)`/g, '<code>$1</code>');
  s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  s = s.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" onclick="return false">$1</a>');
  s = s.replace(/@([\u4e00-\u9fa5A-Za-z][\u4e00-\u9fa5A-Za-z0-9\s]*?)(?=\s|，|。|$|@)/g, (m, n) => {
    const nm = n.trim();
    const known = AF.AGENTS.find(a => a.name === nm || a.en === nm || nm.startsWith(a.name));
    return known ? `<span class="mention">@${known.name}</span>` : m;
  });
  return s;
}
function Markdown({ md }) {
  const lines = md.split('\n');
  const blocks = [];
  let i = 0;
  while (i < lines.length) {
    const ln = lines[i];
    if (/^#{1,3}\s/.test(ln)) {
      const lvl = ln.match(/^#+/)[0].length;
      blocks.push({ tag: 'h' + lvl, html: renderInline(ln.replace(/^#+\s/, '')) });
      i++;
    } else if (/^>\s/.test(ln)) {
      let buf = [];
      while (i < lines.length && /^>\s?/.test(lines[i])) { buf.push(lines[i].replace(/^>\s?/, '')); i++; }
      blocks.push({ tag: 'blockquote', html: buf.map(renderInline).join('<br/>') });
    } else if (/^[-*]\s/.test(ln)) {
      let buf = [];
      while (i < lines.length && /^[-*]\s/.test(lines[i])) { buf.push('<li>' + renderInline(lines[i].replace(/^[-*]\s/, '')) + '</li>'); i++; }
      blocks.push({ tag: 'ul', html: buf.join('') });
    } else if (/^\d+\.\s/.test(ln)) {
      let buf = [];
      while (i < lines.length && /^\d+\.\s/.test(lines[i])) { buf.push('<li>' + renderInline(lines[i].replace(/^\d+\.\s/, '')) + '</li>'); i++; }
      blocks.push({ tag: 'ol', html: buf.join('') });
    } else if (ln.trim() === '') {
      i++;
    } else {
      let buf = [];
      while (i < lines.length && lines[i].trim() !== '' && !/^[#>\-*]|^\d+\./.test(lines[i])) { buf.push(renderInline(lines[i])); i++; }
      blocks.push({ tag: 'p', html: buf.join('<br/>') });
    }
  }
  return blocks.map((b, k) => React.createElement(b.tag, { key: k, dangerouslySetInnerHTML: { __html: b.html } }));
}

/* ---------------- code highlighter — tokenizer → React spans ---------------- */
const KW = new Set(['const','let','var','function','return','import','export','from','if','else','for','while','new','await','async','class','def','self','None','True','False','useState','useSearchParams']);
function tokenize(code) {
  const tokens = [];
  const re = /(\/\/[^\n]*)|("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')|(\b\d+\.?\d*\b)|([A-Za-z_$][\w$]*)|(\s+)|([^\sA-Za-z0-9_$"'\/]+|\/)/g;
  let m;
  while ((m = re.exec(code)) !== null) {
    if (m[1]) tokens.push({ c: 'tok-com', t: m[1] });
    else if (m[2]) tokens.push({ c: 'tok-str', t: m[2] });
    else if (m[3]) tokens.push({ c: 'tok-num', t: m[3] });
    else if (m[4]) {
      const after = code[re.lastIndex];
      if (KW.has(m[4])) tokens.push({ c: 'tok-key', t: m[4] });
      else if (after === '(') tokens.push({ c: 'tok-fn', t: m[4] });
      else tokens.push({ c: null, t: m[4] });
    } else tokens.push({ c: null, t: m[5] || m[6] });
  }
  return tokens;
}
function CodeBlock({ lang, code }) {
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

/* ---------------- message blocks ---------------- */
function Block({ b }) {
  if (b.t === 'md') return <Markdown md={b.md} />;
  if (b.t === 'code') return <CodeBlock lang={b.lang} code={b.code} />;
  if (b.t === 'typing') return <div className="typing"><i /><i /><i /></div>;
  if (b.t === 'file') return (
    <div className="att">
      <div className="att-ic" style={{ background: b.color || '#4f8ed1' }}><Icon name="file" size={19} /></div>
      <div style={{ minWidth: 0 }}><div className="att-name">{b.name}</div><div className="att-meta">{b.meta}</div></div>
      <button className="icon-btn" style={{ marginLeft: 'auto' }}><Icon name="arrowDown" size={16} /></button>
    </div>
  );
  if (b.t === 'image') return (
    <div className="att-img">
      <div className="ph" style={{ background: `linear-gradient(135deg, ${b.color}, ${b.color}99)` }}><Icon name="image" size={30} /></div>
      <div className="cap">{b.label}　{b.meta}</div>
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
        {b.rows.map((r, i) => <div className="artifact-row" key={i}><span className="k">{r[0]}</span><span className="v">{r[1]}</span></div>)}
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

Object.assign(window, { Icon, Avatar, MeAvatar, Markdown, CodeBlock, Block, useState, useRef, useEffect });
