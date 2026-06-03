import React from 'react';
import { AGENTS } from '../data/mock';

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
}

export default function Markdown({ md }: { md: string }) {
  const lines = md.split('\n');
  const blocks: Block[] = [];
  let i = 0;

  while (i < lines.length) {
    const ln = lines[i];
    if (/^#{1,3}\s/.test(ln)) {
      const lvl = ln.match(/^#+/)![0].length;
      blocks.push({ tag: 'h' + lvl, html: renderInline(ln.replace(/^#+\s/, '')) });
      i++;
    } else if (/^>\s/.test(ln)) {
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
      while (i < lines.length && lines[i].trim() !== '' && !/^[#>\-*]|^\d+\./.test(lines[i])) {
        buf.push(renderInline(lines[i]));
        i++;
      }
      blocks.push({ tag: 'p', html: buf.join('<br/>') });
    }
  }

  return (
    <>
      {blocks.map((b, k) =>
        React.createElement(b.tag, {
          key: k,
          dangerouslySetInnerHTML: { __html: b.html },
        })
      )}
    </>
  );
}
