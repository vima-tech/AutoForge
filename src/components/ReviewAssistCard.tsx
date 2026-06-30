import React, { useState, useEffect, useCallback } from 'react';
import Icon from './Icon';
import Markdown from './Markdown';
import {
  getCodeReviewSummary, generateCodeReviewSummary,
  getReleaseNotes, generateReleaseNotes,
} from '../services';

interface Props {
  crId: string;
  /** 当前 CR 是否有可分析的代码改动（无改动/失败/冲突态传 false，隐藏卡片）。 */
  enabled: boolean;
}

interface ReleaseNote { kind: string; headline: string; body: string }

// release_notes 角色输出的 kind → chip 语义变体。
const RN_CHIP: Record<string, string> = {
  feature: 'green', fix: 'red', improvement: 'blue', refactor: 'violet', chore: 'amber',
};

/** 解析 release_notes 的 JSON 原文（容错：解析失败时把原文当 body 显示）。 */
function parseNote(raw: string): ReleaseNote | null {
  const t = raw.trim();
  if (!t) return null;
  try {
    const j = JSON.parse(t.replace(/^```(json)?/i, '').replace(/```$/, '').trim());
    return { kind: String(j.kind ?? ''), headline: String(j.headline ?? ''), body: String(j.body ?? '') };
  } catch {
    return { kind: '', headline: '', body: t };
  }
}

/**
 * 审核辅助卡：两块按需生成的 AI 辅助——
 * ① 代码预审摘要（code_reviewer，Markdown）：进入代码审核前帮审核者抓重点。
 * ② 发布说明（release_notes，面向用户的变更说明）。
 * 二者都「读已生成 / 主动生成」，失败/空态降级，绝不阻塞 Diff 与审核操作。
 */
export default function ReviewAssistCard({ crId, enabled }: Props) {
  const [review, setReview] = useState('');
  const [note, setNote] = useState<ReleaseNote | null>(null);
  const [busyReview, setBusyReview] = useState(false);
  const [busyNote, setBusyNote] = useState(false);
  const [err, setErr] = useState('');

  const load = useCallback(() => {
    let alive = true;
    setReview(''); setNote(null); setErr('');
    getCodeReviewSummary(crId).then(s => { if (alive) setReview(s || ''); }).catch(() => {});
    getReleaseNotes(crId).then(s => { if (alive) setNote(parseNote(s || '')); }).catch(() => {});
    return () => { alive = false; };
  }, [crId]);

  useEffect(() => { if (enabled && crId) return load(); }, [load, enabled, crId]);

  if (!enabled || !crId) return null;

  const genReview = async () => {
    setBusyReview(true); setErr('');
    try { setReview(await generateCodeReviewSummary(crId)); }
    catch (e) { setErr(String(e)); }
    finally { setBusyReview(false); }
  };
  const genNote = async () => {
    setBusyNote(true); setErr('');
    try { setNote(parseNote(await generateReleaseNotes(crId))); }
    catch (e) { setErr(String(e)); }
    finally { setBusyNote(false); }
  };

  return (
    <div className="panel" style={{ margin: '0 clamp(8px, 1.4vw, 24px) 12px', overflow: 'hidden' }}>
      <div className="panel-head" style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '11px 14px' }}>
        <Icon name="search" size={16} style={{ color: 'var(--ember)' }} />
        <span style={{ fontWeight: 700, fontSize: 'var(--text-title)' }}>审核辅助</span>
        <span
          className="chip"
          style={{ fontSize: 'var(--text-micro)', fontFamily: 'var(--font-mono)', color: 'var(--text-3)' }}
          title="由 AI 基于 diff 生成，仅供参考，请独立判断"
        >
          AI 生成
        </span>
      </div>

      <div style={{ padding: '12px 14px', display: 'flex', flexDirection: 'column', gap: 14 }}>
        {err && (
          <div style={{ color: 'var(--red)', fontSize: 'var(--text-control)' }}>{err}</div>
        )}

        {/* ① 代码预审摘要 */}
        <section style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', letterSpacing: '.08em', textTransform: 'uppercase', color: 'var(--text-3)' }}>
              代码预审摘要
            </span>
            <button className="btn btn-sm" style={{ marginLeft: 'auto' }} onClick={genReview} disabled={busyReview}>
              {busyReview ? '生成中…' : review ? '重新生成' : '生成预审摘要'}
            </button>
          </div>
          {review
            ? <div className="bubble doc" style={{ padding: '4px 2px' }}><Markdown md={review} /></div>
            : !busyReview && <div style={{ color: 'var(--text-faint)', fontSize: 'var(--text-control)' }}>尚未生成。点击右上按钮，让 code_reviewer 角色基于本次 diff 生成预审摘要。</div>}
        </section>

        {/* ② 发布说明 */}
        <section style={{ display: 'flex', flexDirection: 'column', gap: 8, borderTop: '1px solid var(--border)', paddingTop: 12 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', letterSpacing: '.08em', textTransform: 'uppercase', color: 'var(--text-3)' }}>
              发布说明
            </span>
            <button className="btn btn-sm" style={{ marginLeft: 'auto' }} onClick={genNote} disabled={busyNote}>
              {busyNote ? '生成中…' : note ? '重新生成' : '生成发布说明'}
            </button>
          </div>
          {note
            ? (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  {note.kind && <span className={`chip ${RN_CHIP[note.kind] ?? 'amber'}`} style={{ fontSize: 'var(--text-micro)', fontFamily: 'var(--font-mono)' }}>{note.kind}</span>}
                  {note.headline && <span style={{ fontWeight: 600 }}>{note.headline}</span>}
                </div>
                {note.body && <div className="bubble doc" style={{ padding: '4px 2px' }}><Markdown md={note.body} /></div>}
              </div>
            )
            : !busyNote && <div style={{ color: 'var(--text-faint)', fontSize: 'var(--text-control)' }}>尚未生成。点击右上按钮，让 release_notes 角色生成面向用户的变更说明。</div>}
        </section>
      </div>
    </div>
  );
}
