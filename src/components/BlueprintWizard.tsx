import React, { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import Icon from './Icon';
import {
  generateProjectBlueprint, applyProjectBlueprint, enqueueBlueprintTasks,
  type Project, type BlueprintResult,
} from '../services';

/**
 * 项目蓝图向导：把一段「大需求」炼成 PRD + 技术规格 + TASKLIST。
 * 两步、显式、人审优先：
 *  1. 输入大需求 → AI 生成蓝图（仅预览，不落盘/不入池）。
 *  2. 预览 → 「写入工作区」落 .autoforge/docs|specs；勾选任务「一键转需求」入 triage 池。
 * 关闭只走 ✕ / Esc（不点遮罩关闭），符合 DESIGN.md 模态规范。
 */
export default function BlueprintWizard({ project, onClose, onApplied }: {
  project: Project; onClose: () => void; onApplied?: () => void;
}) {
  const [step, setStep] = useState<'input' | 'preview'>('input');
  const [brief, setBrief] = useState('');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState('');
  const [result, setResult] = useState<BlueprintResult | null>(null);
  const [picked, setPicked] = useState<boolean[]>([]);
  const [writeTasklistDoc, setWriteTasklistDoc] = useState(true);
  const [applyMsg, setApplyMsg] = useState('');
  const [enqueueMsg, setEnqueueMsg] = useState('');
  const noRepo = !project.repo_path?.trim();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const generate = async () => {
    if (!brief.trim()) { setErr('请先粘贴大需求内容'); return; }
    setBusy(true); setErr('');
    try {
      const r = await generateProjectBlueprint(project.id, brief.trim());
      setResult(r);
      setPicked(r.tasklist.map(() => true));
      setStep('preview');
    } catch (e) { setErr(String(e)); }
    finally { setBusy(false); }
  };

  const apply = async () => {
    if (!result) return;
    setBusy(true); setErr(''); setApplyMsg('');
    try {
      const msg = await applyProjectBlueprint(
        project.id, result.prd_markdown, result.specs,
        writeTasklistDoc ? result.tasklist_markdown : null,
      );
      setApplyMsg(msg);
      onApplied?.();
    } catch (e) { setErr(String(e)); }
    finally { setBusy(false); }
  };

  const enqueue = async () => {
    if (!result) return;
    const tasks = result.tasklist.filter((_, i) => picked[i]);
    if (!tasks.length) { setErr('请至少勾选一个任务'); return; }
    setBusy(true); setErr(''); setEnqueueMsg('');
    try {
      const n = await enqueueBlueprintTasks(project.id, tasks);
      setEnqueueMsg(`已入待整理池 ${n} 条需求`);
    } catch (e) { setErr(String(e)); }
    finally { setBusy(false); }
  };

  const pickedCount = picked.filter(Boolean).length;

  return createPortal(
    <div style={{
      position: 'fixed', inset: 'var(--win-gutter, 0)', borderRadius: 14,
      background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(2px)',
      display: 'flex', alignItems: 'flex-start', justifyContent: 'center', paddingTop: '8vh', zIndex: 1000,
    }}>
      <div className="rise" style={{
        width: 'min(720px, 94%)', maxHeight: '84vh', display: 'flex', flexDirection: 'column',
        background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 16,
        boxShadow: 'var(--shadow-lg)', padding: '18px 20px',
      }}>
        {/* header */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 14 }}>
          <div style={{ width: 30, height: 30, borderRadius: 8, background: 'var(--ember-tint)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Icon name="layers" size={15} style={{ color: 'var(--ember)' }} />
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontWeight: 600, fontSize: 'var(--text-body)' }}>项目蓝图 · {project.name}</div>
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 1 }}>
              把一段大需求炼成 PRD / 技术规格 / 任务清单
            </div>
          </div>
          <button className="icon-btn" onClick={onClose} title="关闭"><Icon name="x" size={15} /></button>
        </div>

        {noRepo && (
          <div className="chip amber" style={{ display: 'block', padding: '8px 12px', marginBottom: 12, lineHeight: 'var(--leading-normal)' }}>
            该项目未配置本地仓库路径，可生成预览，但「写入工作区」会失败。
          </div>
        )}

        <div style={{ overflowY: 'auto', flex: 1, minHeight: 0 }}>
          {step === 'input' ? (
            <>
              <div className="field full" style={{ margin: 0 }}>
                <label>大需求原文</label>
                <textarea
                  rows={10} value={brief} onChange={e => setBrief(e.target.value)} autoFocus
                  placeholder="把这个项目要做的事整段写下来——背景、目标、要支持的能力、约束…越具体，AI 拆出的 PRD / 规格 / 任务越准。"
                  style={{ resize: 'vertical', minHeight: 200, fontFamily: 'var(--font-sans)' }}
                />
              </div>
            </>
          ) : result && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
              {/* PRD */}
              <Section icon="inbox" title="PRD（产品需求文档）" hint="写入 .autoforge/docs/PRD.md">
                <pre style={preStyle}>{result.prd_markdown || '（空）'}</pre>
              </Section>

              {/* specs */}
              <Section icon="layers" title={`技术规格 · ${result.specs.length} 条`} hint="登记到项目规格并写入 .autoforge/specs/">
                {result.specs.length === 0 ? <Empty /> : (
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                    {result.specs.map((s, i) => (
                      <div key={i} className="panel" style={{ padding: '10px 12px' }}>
                        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
                          <span className="chip" style={{ fontSize: 'var(--text-micro)' }}>{s.category}</span>
                          <span style={{ fontSize: 'var(--text-control)', fontWeight: 600 }}>{s.title}</span>
                        </div>
                        <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-2)', whiteSpace: 'pre-line' }}>{s.content_markdown}</div>
                      </div>
                    ))}
                  </div>
                )}
              </Section>

              {/* tasklist */}
              <Section icon="list" title={`任务清单 · ${result.tasklist.length} 条`} hint="勾选后可一键转为待整理需求">
                {result.tasklist.length === 0 ? <Empty /> : (
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                    {result.tasklist.map((t, i) => (
                      <label key={i} className="panel" style={{ display: 'flex', alignItems: 'flex-start', gap: 10, padding: '9px 12px', cursor: 'pointer' }}>
                        <input type="checkbox" checked={picked[i] ?? false}
                          onChange={e => setPicked(p => p.map((v, j) => j === i ? e.target.checked : v))}
                          style={{ marginTop: 3, accentColor: 'var(--ember)' }} />
                        <div style={{ flex: 1, minWidth: 0 }}>
                          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                            <span style={{ fontSize: 'var(--text-control)', fontWeight: 600 }}>{t.title}</span>
                            <span className="chip" style={{ fontSize: 'var(--text-micro)' }}>{t.category}</span>
                            <span className="chip" style={{ fontSize: 'var(--text-micro)' }}>{t.severity}</span>
                          </div>
                          {t.description && <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 2 }}>{t.description}</div>}
                        </div>
                      </label>
                    ))}
                  </div>
                )}
                <label style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 8, fontSize: 'var(--text-label)', color: 'var(--text-2)', cursor: 'pointer' }}>
                  <input type="checkbox" checked={writeTasklistDoc} onChange={e => setWriteTasklistDoc(e.target.checked)} style={{ accentColor: 'var(--ember)' }} />
                  同时把任务清单写入 .autoforge/docs/TASKLIST.md
                </label>
              </Section>

              {(applyMsg || enqueueMsg) && (
                <div className="chip green" style={{ display: 'block', padding: '8px 12px', lineHeight: 'var(--leading-normal)' }}>
                  {applyMsg && <div>✓ {applyMsg}</div>}
                  {enqueueMsg && <div>✓ {enqueueMsg}</div>}
                </div>
              )}
            </div>
          )}
        </div>

        {err && <div style={{ fontSize: 'var(--text-label)', color: 'var(--red)', marginTop: 10 }}>{err}</div>}

        {/* footer actions */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginTop: 14, flexShrink: 0 }}>
          {step === 'preview' && (
            <button className="btn btn-sm btn-ghost" onClick={() => { setStep('input'); setErr(''); }} disabled={busy}>
              <Icon name="code" size={13} />重写大需求
            </button>
          )}
          <div style={{ flex: 1 }} />
          {step === 'input' ? (
            <button className="btn btn-primary" onClick={generate} disabled={busy || !brief.trim()}>
              <Icon name="zap" size={14} />{busy ? '生成中…' : 'AI 生成蓝图'}
            </button>
          ) : (
            <>
              <button className="btn" onClick={enqueue} disabled={busy || pickedCount === 0}
                title="把勾选任务作为需求入待整理池">
                <Icon name="inbox" size={14} />一键转需求{pickedCount ? `（${pickedCount}）` : ''}
              </button>
              <button className="btn btn-primary" onClick={apply} disabled={busy || noRepo}>
                <Icon name="check" size={14} />{busy ? '写入中…' : '写入工作区'}
              </button>
            </>
          )}
        </div>
      </div>
    </div>,
    document.querySelector('.os-window') ?? document.body,
  );
}

const preStyle: React.CSSProperties = {
  margin: 0, maxHeight: 220, overflow: 'auto', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
  background: 'var(--bg-3)', border: '1px solid var(--border)', borderRadius: 9, padding: '10px 12px',
  fontSize: 'var(--text-label)', color: 'var(--text-2)', fontFamily: 'var(--font-sans)', lineHeight: 'var(--leading-relaxed)',
};

function Section({ icon, title, hint, children }: { icon: string; title: string; hint?: string; children: React.ReactNode }) {
  return (
    <div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
        <Icon name={icon} size={15} style={{ color: 'var(--ember)' }} />
        <span style={{ fontSize: 'var(--text-control)', fontWeight: 700 }}>{title}</span>
        {hint && <span style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)' }}>· {hint}</span>}
      </div>
      {children}
    </div>
  );
}

function Empty() {
  return <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-faint)', padding: '6px 0' }}>（AI 未生成此部分）</div>;
}
