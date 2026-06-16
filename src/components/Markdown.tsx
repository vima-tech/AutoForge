import React from 'react';
import { AGENTS } from '../data/mock';
import { tokenize } from './Block';
import { highlightHtml } from './highlight';

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

function renderInline(text: string): string {
  let s = text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  s = s.replace(/`([^`]+)`/g, '<code>$1</code>');
  s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  s = s.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" onClick="return false">$1</a>');
  s = s.replace(/@([一-龥A-Za-z][一-龥A-Za-z0-9\s]*?)(?=\s|，|。|$|@)/g, (_m, n) => {
    const nm = n.trim();
    const known = AGENTS.find(a => a.name === nm || a.en === nm || nm.startsWith(a.name));
    return known ? `<span class="mention">@${known.name}</span>` : _m;
  });
  return s;
}

interface Block {
  tag: string;
  html: string;
  cls?: string;
}

export default function Markdown({ md, highlight }: { md: string; highlight?: string }) {
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
      blocks.push({ tag: 'h' + lvl, html: renderInline(ln.replace(/^#+\s/, '')) });
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
        text.split(/(<br\s*\/?>)/i).map((p, j) => j % 2 === 0 ? renderInline(p) : '<br/>').join('');

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
      blocks.push({ tag: 'div', html });
    } else if (/^>\s?/.test(ln)) {
      const buf: string[] = [];
      while (i < lines.length && /^>\s?/.test(lines[i])) {
        buf.push(lines[i].replace(/^>\s?/, ''));
        i++;
      }
      blocks.push({ tag: 'blockquote', html: buf.map(renderInline).join('<br/>') });
    } else if (/^[-*]\s/.test(ln)) {
      const buf: string[] = [];
      while (i < lines.length && /^[-*]\s/.test(lines[i])) {
        buf.push('<li>' + renderInline(lines[i].replace(/^[-*]\s/, '')) + '</li>');
        i++;
      }
      blocks.push({ tag: 'ul', html: buf.join('') });
    } else if (/^\d+\.\s/.test(ln)) {
      const buf: string[] = [];
      while (i < lines.length && /^\d+\.\s/.test(lines[i])) {
        buf.push('<li>' + renderInline(lines[i].replace(/^\d+\.\s/, '')) + '</li>');
        i++;
      }
      blocks.push({ tag: 'ol', html: buf.join('') });
    } else if (ln.trim() === '') {
      i++;
    } else {
      const buf: string[] = [];
      const stop = (l: string) => l.trim() === '' || /^[#>\-*]|^\d+\.|\||^\s*(```|~~~)/.test(l);
      while (i < lines.length && !stop(lines[i])) {
        buf.push(renderInline(lines[i]));
        i++;
      }
      // Safety: if nothing was consumed (stop matched immediately), skip the
      // line to avoid an infinite loop on unrecognised patterns.
      if (buf.length === 0) { i++; }
      else blocks.push({ tag: 'p', html: buf.join('<br/>') });
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
