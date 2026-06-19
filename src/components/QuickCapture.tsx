import React, { useState, useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';
import Icon from './Icon';
import Select from './Select';
import { listProjects, submitIssue, transcribeAudio, type Project } from '../services';
import { RealtimeAsr } from '../lib/realtimeAsr';

// 音频 blob → base64（剥离 data URL 前缀）。
function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const fr = new FileReader();
    fr.onload = () => resolve(String(fr.result).split(',')[1] ?? '');
    fr.onerror = () => reject(new Error('音频读取失败'));
    fr.readAsDataURL(blob);
  });
}

/**
 * 全局速录：随手把一个念头零结构地丢进「待整理池」（status=triage，不自动分析）。
 * 这是传送带的「扔念头」入口——只一个文本框，回车即入池，结构化交给 triage Agent。
 */
export default function QuickCapture({ onClose }: { onClose: () => void }) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectId, setProjectId] = useState('');
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState('');
  const [ok, setOk] = useState(false);
  const [streaming, setStreaming] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [transcribing, setTranscribing] = useState(false);
  const taRef = useRef<HTMLTextAreaElement | null>(null);
  const fileRef = useRef<HTMLInputElement | null>(null);
  const rtRef = useRef<RealtimeAsr | null>(null);
  // 实时识别的文本拼接：base(起录时已有文本) + committed(已定句) + partial(当前增量句)。
  const baseRef = useRef('');
  const committedRef = useRef('');
  const partialRef = useRef('');

  useEffect(() => {
    listProjects().then(ps => {
      setProjects(ps);
      const active = ps.find(p => p.status === 'active') ?? ps[0];
      if (active) setProjectId(active.id);
    }).catch(e => setErr(String(e)));
    setTimeout(() => taRef.current?.focus(), 50);
    return () => { void rtRef.current?.stop(); };
  }, []);

  // 实时语音（阿里 DashScope 流式）：边说边把结果写进文本框。
  const startRealtime = async () => {
    if (streaming || connecting) return;
    setErr('');
    setConnecting(true); // 点击即给反馈，不等建链完成（麦克风授权 + DashScope WS 握手有延迟）。
    baseRef.current = text ? text + (text.endsWith(' ') ? '' : ' ') : '';
    committedRef.current = ''; partialRef.current = '';
    const rt = new RealtimeAsr();
    rtRef.current = rt;
    try {
      await rt.start((t, isFinal) => {
        if (isFinal) { committedRef.current += t; partialRef.current = ''; }
        else { partialRef.current = t; }
        setText(baseRef.current + committedRef.current + partialRef.current);
      });
      setStreaming(true);
    } catch (e) {
      rtRef.current = null;
      setErr(String(e) + '；可改用「音频文件」');
    } finally {
      setConnecting(false);
    }
  };
  const stopRealtime = async () => {
    setStreaming(false);
    const rt = rtRef.current; rtRef.current = null;
    await rt?.stop();
    setTimeout(() => taRef.current?.focus(), 30);
  };

  // 音频文件转写（批量回退，走 OpenAI 兼容 /audio/transcriptions）。
  const transcribeFile = async (blob: Blob) => {
    setTranscribing(true); setErr('');
    try {
      const b64 = await blobToBase64(blob);
      const r = await transcribeAudio(b64, blob.type || 'audio/webm');
      setText(prev => (prev ? prev + ' ' : '') + r.text);
      setTimeout(() => taRef.current?.focus(), 30);
    } catch (e) { setErr(String(e)); }
    finally { setTranscribing(false); }
  };
  const onPickFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const f = e.target.files?.[0]; if (f) void transcribeFile(f); e.target.value = '';
  };

  const submit = async () => {
    // 录音进行中点速录：先停掉实时识别（落定最后一句），再入池。
    if (streaming || rtRef.current) await stopRealtime();
    if (!text.trim()) return;
    if (!projectId) { setErr('请先选择归属项目'); return; }
    setBusy(true); setErr('');
    try {
      const first = text.trim().split(/[\n。.!?！？]/)[0]?.trim() ?? text.trim();
      await submitIssue({
        project_id: projectId,
        title: first.length > 30 ? first.slice(0, 30) : first,
        description: text.trim(),
        source_type: 'quickcapture',
        mode: 'triage',
      });
      setOk(true); setText('');
      setTimeout(() => taRef.current?.focus(), 30);
      setTimeout(() => setOk(false), 1800);
    } catch (e) { setErr(String(e)); }
    finally { setBusy(false); }
  };

  // Cmd/Ctrl+Enter 或 Enter（无 shift）提交。
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); void submit(); }
    if (e.key === 'Escape') onClose();
  };

  return createPortal(
    <div
      style={{
        position: 'fixed', inset: 'var(--win-gutter, 0)', borderRadius: 14,
        background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(2px)',
        display: 'flex', alignItems: 'flex-start', justifyContent: 'center', paddingTop: '14vh', zIndex: 1000,
      }}
    >
      <div
        className="rise"
        style={{
          width: 'min(540px, 92%)', background: 'var(--bg-2)',
          border: '1px solid var(--border-strong)', borderRadius: 16,
          boxShadow: 'var(--shadow-lg)', padding: '18px 20px',
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 14 }}>
          <div style={{ width: 30, height: 30, borderRadius: 8, background: 'var(--ember-tint)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Icon name="zap" size={15} style={{ color: 'var(--ember)' }} />
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ fontWeight: 600, fontSize: 'var(--text-body)' }}>速录念头</div>
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 1 }}>随手记，不用想结构 — 入「待整理池」交给 AI 炼成需求</div>
          </div>
          <button className="icon-btn" onClick={onClose} title="关闭"><Icon name="x" size={15} /></button>
        </div>

        <textarea
          ref={taRef} value={text} onChange={e => setText(e.target.value)} onKeyDown={onKey}
          rows={3} placeholder="把脑子里冒出来的东西吐进来，多碎都行…（Enter 提交，Shift+Enter 换行）"
          style={{ width: '100%', boxSizing: 'border-box', background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 9, padding: '10px 12px', color: 'var(--text)', fontSize: 'var(--text-body)', fontFamily: 'var(--font-sans)', resize: 'vertical', outline: 'none', marginBottom: 10 }}
        />

        {/* 语音录入（实时优先：阿里 DashScope 流式，边说边出字） */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
          {!streaming ? (
            <button className="btn btn-sm" onClick={startRealtime} disabled={transcribing || connecting} title="实时语音录入（边说边转写）">
              <Icon name="mic" size={13} />{connecting ? '连接中…' : '实时语音'}
            </button>
          ) : (
            <button className="btn btn-sm btn-danger" onClick={stopRealtime}>
              <Icon name="pause" size={13} />停止
            </button>
          )}
          <button className="btn btn-sm btn-ghost" onClick={() => fileRef.current?.click()} disabled={streaming || connecting || transcribing} title="选择音频文件转写">
            <Icon name="paperclip" size={13} />音频文件
          </button>
          <input ref={fileRef} type="file" accept="audio/*" style={{ display: 'none' }} onChange={onPickFile} />
          {connecting && <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5, fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}><span className="dot amber" />连接中…</span>}
          {streaming && <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5, fontSize: 'var(--text-label)', color: 'var(--red)', fontFamily: 'var(--font-mono)' }}><span className="dot amber" />聆听中…</span>}
          {transcribing && <span style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>转写中…</span>}
          {!streaming && !transcribing && err && <span style={{ fontSize: 'var(--text-label)', color: 'var(--red)', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={err}>{err}</span>}
        </div>

        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <div style={{ flex: 1, minWidth: 0 }}>
            <Select value={projectId} onChange={setProjectId}
              options={projects.map(p => ({ value: p.id, label: p.name }))} placeholder="归属项目" />
          </div>
          {ok && <span style={{ fontSize: 'var(--text-label)', color: 'var(--green)', fontFamily: 'var(--font-mono)' }}>✓ 已入池</span>}
          <button className="btn btn-primary" onClick={submit} disabled={busy || !text.trim()}>
            <Icon name="send" size={14} />{busy ? '入池中…' : '速录'}
          </button>
        </div>
      </div>
    </div>,
    document.querySelector('.os-window') ?? document.body,
  );
}
