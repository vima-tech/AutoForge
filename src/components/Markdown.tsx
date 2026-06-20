import React from 'react';
import { useMemo } from 'react';
import { useAgents, type Agent } from '../agents-store';
import { tokenize } from './Block';
import { highlightHtml } from './highlight';

// 预编译的 @提及索引：exact 走 O(1) 命中（name / name_en → 展示名），
// 找不到再按「名字长者优先」做前缀兜底（避免短名 "测" 抢先盖过 "测试"）。
// 空名一律跳过，否则 nm.startsWith('') 会把每个 @ 都误判成提及。
interface MentionIndex {
  exact: Map<string, string>;
  prefixes: string[];
}

function buildMentionIndex(agents: Agent[]): MentionIndex {
  const exact = new Map<string, string>();
  const prefixes: string[] = [];
  for (const a of agents) {
    if (a.name) {
      exact.set(a.name, a.name);
      prefixes.push(a.name);
    }
    if (a.name_en) exact.set(a.name_en, a.name);
  }
  prefixes.sort((x, y) => y.length - x.length);
  return { exact, prefixes };
}

function resolveMention(nm: string, idx: MentionIndex): string | null {
  const hit = idx.exact.get(nm);
  if (hit) return hit;
  for (const name of idx.prefixes) if (nm.startsWith(name)) return name;
  return null;
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

// Highlight code into an HTML string, reusing the same tokenizer/classes as the
// standalone `code` block so embedded fences look identical.
function highlightCode(code: string): string {
  return tokenize(code)
    .map(tk => (tk.c ? `<span class="${tk.c}">${escapeHtml(tk.t)}</span>` : escapeHtml(tk.t)))
    .join('');
}

function renderInline(text: string, idx: MentionIndex): string {
  let s = text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  s = s.replace(/`([^`]+)`/g, '<code>$1</code>');
  s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  s = s.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" onClick="return false">$1</a>');
  s = s.replace(/@([一-龥A-Za-z][一-龥A-Za-z0-9\s]*?)(?=\s|，|。|$|@)/g, (_m, n) => {
    // 基于 DB 真实 Agent 列表匹配；列表未就绪或无匹配则按普通文本渲染。
    const name = resolveMention(n.trim(), idx);
    return name ? `<span class="mention">@${name}</span>` : _m;
  });
  return s;
}

interface Block {
  tag: string;
  html: string;
  cls?: string;
}

export default function Markdown({ md, highlight }: { md: string; highlight?: string }) {
  const agents = useAgents();
  // agents 引用来自 store cache，仅在列表真正刷新时变化 → 索引随之重建。
  const mentionIdx = useMemo(() => buildMentionIndex(agents), [agents]);
  const ri = (t: string) => renderInline(t, mentionIdx);
  const lines = md.split('\n');
  const blocks: Block[] = [];
  let i = 0;

  while (i < lines.length) {
    const ln = lines[i];
    const fence = ln.match(/^(\s*)(```+|~~~+)\s*([^\s`~]*)/);
    if (fence) {
      // Fenced code block: ``` or ~~~ (optional language), closed by a matching
      // fence of the same character. An unterminated fence runs to the end.
      const marker = fence[2][0];          // ` or ~
      const lang = (fence[3] || '').trim();
      const codeLines: string[] = [];
      i++;
      while (i < lines.length) {
        const cl = lines[i].match(/^\s*(```+|~~~+)\s*$/);
        if (cl && cl[1][0] === marker) { i++; break; }
        codeLines.push(lines[i]);
        i++;
      }
      const inner =
        `<div class="codeblock-head"><span class="lang">${escapeHtml(lang || 'code')}</span></div>` +
        `<pre><code>${highlightCode(codeLines.join('\n'))}</code></pre>`;
      blocks.push({ tag: 'div', cls: 'codeblock', html: inner });
    } else if (/^#{1,6}\s/.test(ln)) {
      const lvl = Math.min(ln.match(/^#+/)![0].length, 6);
      blocks.push({ tag: 'h' + lvl, html: ri(ln.replace(/^#+\s/, '')) });
      i++;
    } else if (/^---+$|^\*\*\*+$|^___+$/.test(ln.trim())) {
      // Horizontal rule: ---  ***  ___
      blocks.push({ tag: 'hr', html: '' });
      i++;
    } else if (/^\|/.test(ln)) {
      // Collect all consecutive pipe-delimited lines, render as a proper <table>
      const rows: string[] = [];
      while (i < lines.length && /^\|/.test(lines[i])) { rows.push(lines[i]); i++; }

      const parseRow = (line: string) =>
        line.replace(/^\|/, '').replace(/\|$/, '').split('|').map(c => c.trim());
      const isSep = (line: string) => /^\|[\s|:-]+\|$/.test(line.trim());
      // Handle literal <br> / <br/> in cell text before HTML-escaping the rest
      const renderCell = (text: string) =>
        text.split(/(<br\s*\/?>)/i).map((p, j) => j % 2 === 0 ? ri(p) : '<br/>').join('');

      let html = '<table class="md-table"><thead>';
      let bodyStarted = false;
      for (const row of rows) {
        if (isSep(row)) {
          html += '</thead><tbody>';
          bodyStarted = true;
          continue;
        }
        const cells = parseRow(row);
        const tag = bodyStarted ? 'td' : 'th';
        html += '<tr>' + cells.map(c => `<${tag}>${renderCell(c)}</${tag}>`).join('') + '</tr>';
      }
      html += bodyStarted ? '</tbody></table>' : '</thead></table>';
      // 包一层可横向滚动容器：宽表格（th 不换行）在文档流里自身滚动，
      // 不会把气泡撑过可用宽度而压到右侧大纲（.doc-toc）上。
      blocks.push({ tag: 'div', cls: 'md-table-wrap', html });
    } else if (/^>\s?/.test(ln)) {
      const buf: string[] = [];
      while (i < lines.length && /^>\s?/.test(lines[i])) {
        buf.push(lines[i].replace(/^>\s?/, ''));
        i++;
      }
      blocks.push({ tag: 'blockquote', html: buf.map(ri).join('<br/>') });
    } else if (/^[-*]\s/.test(ln)) {
      const buf: string[] = [];
      while (i < lines.length && /^[-*]\s/.test(lines[i])) {
        buf.push('<li>' + ri(lines[i].replace(/^[-*]\s/, '')) + '</li>');
        i++;
      }
      blocks.push({ tag: 'ul', html: buf.join('') });
    } else if (/^\d+\.\s/.test(ln)) {
      const buf: string[] = [];
      while (i < lines.length && /^\d+\.\s/.test(lines[i])) {
        buf.push('<li>' + ri(lines[i].replace(/^\d+\.\s/, '')) + '</li>');
        i++;
      }
      blocks.push({ tag: 'ol', html: buf.join('') });
    } else if (ln.trim() === '') {
      i++;
    } else {
      // Paragraph. We only reach here because `ln` matched none of the block
      // openers above (heading/list/blockquote all require a trailing space, the
      // fence/table/hr their own shapes), so it is genuine prose — consume it
      // unconditionally. `stop` then bounds how far the paragraph extends; using
      // it to gate the *first* line would silently drop lines like `#tag` or
      // `*.png` (hash/star without the space that makes them a block), which is
      // exactly how a `#引用` chip's text used to vanish from the bubble.
      const stop = (l: string) => l.trim() === '' || /^#{1,6}\s|^>\s?|^[-*]\s|^\d+\.\s|\||^\s*(```|~~~)/.test(l);
      const buf: string[] = [ri(ln)];
      i++;
      while (i < lines.length && !stop(lines[i])) {
        buf.push(ri(lines[i]));
        i++;
      }
      blocks.push({ tag: 'p', html: buf.join('<br/>') });
    }
  }

  return (
    <>
      {blocks.map((b, k) =>
        b.tag === 'hr'
          ? React.createElement('hr', { key: k })
          : React.createElement(b.tag, {
              key: k,
              ...(b.cls ? { className: b.cls } : {}),
              dangerouslySetInnerHTML: { __html: highlightHtml(b.html, highlight) },
            })
      )}
    </>
  );
}
