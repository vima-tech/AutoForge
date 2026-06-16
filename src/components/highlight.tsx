import React from 'react';

// Shared search-term highlighting helpers. Matching is case-insensitive and
// literal (the term is treated as plain text, never a pattern). Highlights are
// wrapped in <mark class="search-mark">, themed via CSS variables in index.css.

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/**
 * Highlight occurrences of `term` inside a plain-text string, returning React
 * nodes. Returns the original string untouched when there is no term or match.
 */
export function highlightText(text: string, term?: string): React.ReactNode {
  const needle = term?.trim().toLowerCase();
  if (!needle) return text;
  const lower = text.toLowerCase();
  let idx = lower.indexOf(needle);
  if (idx === -1) return text;

  const out: React.ReactNode[] = [];
  let from = 0;
  let key = 0;
  while (idx !== -1) {
    if (idx > from) out.push(text.slice(from, idx));
    out.push(
      <mark key={key++} className="search-mark">{text.slice(idx, idx + needle.length)}</mark>,
    );
    from = idx + needle.length;
    idx = lower.indexOf(needle, from);
  }
  if (from < text.length) out.push(text.slice(from));
  return out;
}

/**
 * Highlight occurrences of `term` inside an already-rendered HTML string,
 * touching only text segments (never tag names or attributes). Used by the
 * Markdown renderer, which emits HTML via dangerouslySetInnerHTML.
 */
export function highlightHtml(html: string, term?: string): string {
  const t = term?.trim();
  if (!t) return html;
  // Text inside the HTML is HTML-escaped, so escape the needle the same way
  // before building the matcher (identity for typical alphanumeric/CJK terms).
  const re = new RegExp(escapeRegex(escapeHtml(t)), 'gi');
  return html
    .split(/(<[^>]*>)/)
    .map(seg =>
      seg.startsWith('<')
        ? seg
        : seg.replace(re, m => `<mark class="search-mark">${m}</mark>`),
    )
    .join('');
}
