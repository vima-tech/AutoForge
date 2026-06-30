import React, { useState, useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';
import Icon from './Icon';
import Select from './Select';
import { listProjects, submitIssue, importIssueAttachment, type Project } from '../services';
import { RealtimeAsr } from '../lib/realtimeAsr';
import { registerVoiceSurface } from '../lib/voiceInput';
import MeetingUpload from './MeetingUpload';
import AttachmentBar, { fileToUpload } from './AttachmentBar';

/**
 * 全局速录：随手把一个念头零结构地丢进「待整理池」（status=triage，不自动分析）。
 * 这是传送带的「扔念头」入口——只一个文本框，回车即入池，结构化交给 triage Agent。
 */
export default function QuickCapture({ onClose, autoVoice }: { onClose: () => void; autoVoice?: boolean }) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectId, setProjectId] = useState('');
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState('');
  const [ok, setOk] = useState(false);
  const [streaming, setStreaming] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [meetingOpen, setMeetingOpen] = useState(false);
  const [files, setFiles] = useState<File[]>([]);
  const taRef = useRef<HTMLTextAreaElement | null>(null);
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
      }, () => {
        // 麦克风就绪、开始收音（后端握手可能仍在进行）：立即切到录音中，结束「连接中」。
        setConnecting(false);
        setStreaming(true);
      });
    } catch (e) {
      rtRef.current = null;
      setStreaming(false);
      setErr(String(e) + '；可改用「会议录音上传」');
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

  // 把当前的开/关录音逻辑镜像进 ref，供全局语音快捷键调用最新闭包（避免登记时捕获旧 text）。
  const toggleVoiceRef = useRef<() => void>(() => {});
  toggleVoiceRef.current = () => { if (rtRef.current) void stopRealtime(); else void startRealtime(); };

  // 登记为活跃语音面：速录念头打开时，全局语音快捷键切换它的录音。
  // autoVoice：经语音快捷键兜底打开（无其它语音面）时，挂载后立即起录。
  useEffect(() => {
    const unregister = registerVoiceSurface(() => toggleVoiceRef.current());
    if (autoVoice) setTimeout(() => { void startRealtime(); }, 80);
    return unregister;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const submit = async () => {
    // 录音进行中点速录：先停掉实时识别（落定最后一句），再入池。
    if (streaming || rtRef.current) await stopRealtime();
    if (!text.trim()) return;
    if (!projectId) { setErr('请先选择归属项目'); return; }
    setBusy(true); setErr('');
    try {
      const first = text.trim().split(/[\n。.!?！？]/)[0]?.trim() ?? text.trim();
      const issue = await submitIssue({
        project_id: projectId,
        title: first.length > 30 ? first.slice(0, 30) : first,
        description: text.trim(),
        source_type: 'quickcapture',
        mode: 'triage',
      });
      // 两阶段：需求入池后把暂存文件逐个挂到该需求（图片可供 vision 分析）。
      for (const f of files) {
        try { await importIssueAttachment({ issue_id: issue.id, ...(await fileToUpload(f)) }); }
        catch (e) { console.warn('附件上传失败', f.name, e); }
      }
      setOk(true); setText(''); setFiles([]);
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
            <button className="btn btn-sm" onClick={startRealtime} disabled={connecting} title="实时语音录入（边说边转写）">
              <Icon name="mic" size={13} />{connecting ? '连接中…' : '实时语音'}
            </button>
          ) : (
            <button className="btn btn-sm btn-danger" onClick={stopRealtime}>
              <Icon name="pause" size={13} />停止
            </button>
          )}
          <button className="btn btn-sm btn-ghost" onClick={() => setMeetingOpen(true)} disabled={streaming || connecting} title="上传整场会议录音，AI 提炼纪要并拆解需求">
            <Icon name="mic" size={13} />会议录音上传
          </button>
          {connecting && <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5, fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}><span className="dot amber" />连接中…</span>}
          {streaming && <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5, fontSize: 'var(--text-label)', color: 'var(--red)', fontFamily: 'var(--font-mono)' }}><span className="dot amber" />聆听中…</span>}
          {!streaming && err && <span style={{ fontSize: 'var(--text-label)', color: 'var(--red)', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={err}>{err}</span>}
        </div>

        {/* 图片/附件：暂存模式，入池后随需求一起上传 */}
        <div style={{ marginBottom: 12 }}>
          <AttachmentBar staged={files} onStaged={setFiles} />
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
      {meetingOpen && (
        <MeetingUpload projects={projects} defaultProjectId={projectId} onClose={() => setMeetingOpen(false)} />
      )}
    </div>,
    document.querySelector('.os-window') ?? document.body,
  );
}
