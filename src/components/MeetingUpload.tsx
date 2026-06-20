import React, { useState, useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';
import Icon from './Icon';
import Select from './Select';
import {
  transcribeRecordingSegment, transcribeRecordingFile, analyzeMeeting, saveMeetingDoc, submitIssue,
  type Project, type MeetingIssue,
} from '../services';
import { fileToPcmSegments } from '../lib/audioChunks';

// 文件 → base64（剥离 data URL 前缀），用于后端兜底解码。
function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const fr = new FileReader();
    fr.onload = () => resolve(String(fr.result).split(',')[1] ?? '');
    fr.onerror = () => reject(new Error('文件读取失败'));
    fr.readAsDataURL(file);
  });
}

type Phase = 'idle' | 'working' | 'review' | 'submitting' | 'done';
type EditableReq = MeetingIssue & { selected: boolean };

const CATEGORY_OPTS = [
  { value: 'Feature', label: 'Feature 功能' },
  { value: 'Bug', label: 'Bug 缺陷' },
  { value: 'Improvement', label: 'Improvement 改进' },
  { value: 'Debt', label: 'Debt 技术债' },
];
const SEVERITY_OPTS = [
  { value: 'critical', label: 'critical 紧急' },
  { value: 'high', label: 'high 高' },
  { value: 'medium', label: 'medium 中' },
  { value: 'low', label: 'low 低' },
];

// 从纪要 Markdown 中取首个标题作默认会议标题。
function deriveTitle(summaryMd: string): string {
  const line = summaryMd.split('\n').map(l => l.trim()).find(l => /^#{1,3}\s+/.test(l));
  const t = line ? line.replace(/^#{1,3}\s+/, '').trim() : '';
  if (t) return t.slice(0, 40);
  return `会议纪要 ${new Date().toISOString().slice(0, 10)}`;
}

/**
 * 会议录音上传：长音频前端分片转写 → AI 提炼纪要 + 拆解需求 → 人审勾选/编辑 →
 * 纪要存项目 .autoforge/docs/meetings/，选中需求逐条入「待整理池」（triage）。
 */
export default function MeetingUpload({
  projects, defaultProjectId, onClose,
}: {
  projects: Project[];
  defaultProjectId: string;
  onClose: () => void;
}) {
  const [projectId, setProjectId] = useState(defaultProjectId);
  const [phase, setPhase] = useState<Phase>('idle');
  const [statusText, setStatusText] = useState('');
  const [err, setErr] = useState('');
  const [fileName, setFileName] = useState('');
  const [transcript, setTranscript] = useState('');
  const [title, setTitle] = useState('');
  const [summary, setSummary] = useState('');
  const [reqs, setReqs] = useState<EditableReq[]>([]);
  const [savedPath, setSavedPath] = useState('');
  const [submittedCount, setSubmittedCount] = useState(0);
  const fileRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const busy = phase === 'working' || phase === 'submitting';

  const onPick = (e: React.ChangeEvent<HTMLInputElement>) => {
    const f = e.target.files?.[0];
    e.target.value = '';
    if (f) void process(f);
  };
  const onDrop = (e: React.DragEvent) => {
    e.preventDefault();
    if (busy) return;
    const f = e.dataTransfer.files?.[0];
    if (f) void process(f);
  };

  // 录音 → 分片转写 → AI 分析，进入复盘。
  const process = async (file: File) => {
    setErr(''); setFileName(file.name); setPhase('working');
    try {
      setStatusText('解码音频…');

      // 主路径：浏览器 WebView 解码（覆盖各格式含 Opus）→ 前端分段。
      // 兜底：WebView 缺解码插件解不出时，把原始文件交后端 symphonia 解码（mp3/m4a/wav/flac/ogg）。
      let segments: string[] | null = null;
      let total = 0;
      try {
        segments = await fileToPcmSegments(file, info => {
          total = info.total;
          setStatusText(`音频约 ${Math.round(info.durationSec / 60)} 分钟，分 ${info.total} 段转写…`);
        });
      } catch {
        segments = null; // 走后端兜底
      }

      let fullText: string;
      let failed = 0;
      if (segments) {
        // 逐段转写（顺序，避免并发触发服务限流）；个别段失败跳过不中断整体。
        const parts: string[] = [];
        for (let i = 0; i < segments.length; i++) {
          setStatusText(`转写中 ${i + 1}/${total || segments.length} 段…`);
          try {
            const t = await transcribeRecordingSegment(segments[i]);
            if (t.trim()) parts.push(t.trim());
          } catch {
            failed++;
          }
        }
        fullText = parts.join('\n');
      } else {
        if (file.size > 100 * 1024 * 1024) {
          throw new Error('浏览器无法解码该格式，且文件超过 100MB 无法后端解码；请转成 WAV/MP3 或压缩后再试');
        }
        setStatusText('浏览器无法解码该格式，改用内置解码器转写（可能稍慢）…');
        const b64 = await fileToBase64(file);
        fullText = await transcribeRecordingFile(b64, file.type || undefined);
      }

      if (!fullText.trim()) {
        throw new Error('未能转写出任何文本（录音可能为空/过短/为 Opus 等不受支持编码，或语音模型未正确配置）');
      }
      setTranscript(fullText);

      setStatusText(failed > 0 ? `AI 提炼会议纪要…（${failed} 段转写失败已跳过）` : 'AI 提炼会议纪要、拆解需求…');
      const analysis = await analyzeMeeting(fullText);
      setSummary(analysis.summary_md);
      setTitle(deriveTitle(analysis.summary_md));
      setReqs(analysis.issues.map(r => ({ ...r, selected: true })));
      setPhase('review');
    } catch (e) {
      setErr(String(e instanceof Error ? e.message : e));
      setPhase('idle');
    }
  };

  const patchReq = (i: number, patch: Partial<EditableReq>) =>
    setReqs(rs => rs.map((r, idx) => (idx === i ? { ...r, ...patch } : r)));

  const selectedCount = reqs.filter(r => r.selected).length;

  // 确认：纪要落盘 + 选中需求逐条入池。
  const confirm = async () => {
    if (!projectId) { setErr('请先选择归属项目'); return; }
    if (!title.trim()) { setErr('请填写会议标题'); return; }
    setErr(''); setPhase('submitting');
    try {
      setStatusText('保存会议纪要…');
      const path = await saveMeetingDoc(projectId, title.trim(), summary, transcript);
      setSavedPath(path);

      const chosen = reqs.filter(r => r.selected && r.title.trim());
      let done = 0;
      for (const r of chosen) {
        setStatusText(`入池需求 ${done + 1}/${chosen.length}…`);
        await submitIssue({
          project_id: projectId,
          title: r.title.trim().slice(0, 30),
          description: r.description.trim(),
          category: r.category,
          severity: r.severity,
          source_type: 'meeting',
          mode: 'triage',
        });
        done++;
      }
      setSubmittedCount(done);
      setPhase('done');
    } catch (e) {
      setErr(String(e instanceof Error ? e.message : e));
      setPhase('review');
    }
  };

  const label = (t: string) => (
    <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)', textTransform: 'uppercase', letterSpacing: '.08em', marginBottom: 6 }}>{t}</div>
  );

  return createPortal(
    <div style={{
      position: 'fixed', inset: 'var(--win-gutter, 0)', borderRadius: 14,
      background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(2px)',
      display: 'flex', alignItems: 'flex-start', justifyContent: 'center', paddingTop: '8vh', zIndex: 1100,
    }}>
      <div className="rise" style={{
        width: 'min(720px, 94%)', maxHeight: '82vh', display: 'flex', flexDirection: 'column',
        background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 16,
        boxShadow: 'var(--shadow-lg)',
      }}>
        {/* 头部 */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '16px 20px', borderBottom: '1px solid var(--border)' }}>
          <div style={{ width: 30, height: 30, borderRadius: 8, background: 'var(--ember-tint)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <Icon name="mic" size={15} style={{ color: 'var(--ember)' }} />
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ fontWeight: 600, fontSize: 'var(--text-body)' }}>会议录音上传</div>
            <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', marginTop: 1 }}>转写整场会议 → AI 提炼纪要、拆解需求 → 你审定后入池</div>
          </div>
          <button className="icon-btn" onClick={onClose} title="关闭（Esc）"><Icon name="x" size={15} /></button>
        </div>

        <div style={{ padding: '16px 20px', overflowY: 'auto' }}>
          {/* 项目选择（始终可见） */}
          <div style={{ marginBottom: 14 }}>
            {label('归属项目')}
            <Select value={projectId} onChange={setProjectId}
              options={projects.map(p => ({ value: p.id, label: p.name }))} placeholder="选择项目" />
          </div>

          {/* idle：上传区 */}
          {phase === 'idle' && (
            <div
              onClick={() => fileRef.current?.click()}
              onDragOver={e => e.preventDefault()}
              onDrop={onDrop}
              style={{
                border: '1px dashed var(--border-strong)', borderRadius: 12, padding: '28px 20px',
                textAlign: 'center', cursor: 'pointer', background: 'var(--bg-3)',
              }}
            >
              <Icon name="paperclip" size={20} style={{ color: 'var(--text-3)' }} />
              <div style={{ marginTop: 8, fontSize: 'var(--text-body)', color: 'var(--text)' }}>点击选择或拖入会议录音文件</div>
              <div style={{ marginTop: 4, fontSize: 'var(--text-label)', color: 'var(--text-3)' }}>支持 MP3 / M4A·AAC / WAV / FLAC / OGG / WebM；长音频自动分段转写</div>
              <input ref={fileRef} type="file" accept="audio/*,.mp3,.m4a,.aac,.wav,.flac,.ogg,.oga,.opus,.webm" style={{ display: 'none' }} onChange={onPick} />
            </div>
          )}

          {/* working / submitting：进度 */}
          {busy && (
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '24px 4px', color: 'var(--text-2)' }}>
              <span className="dot amber" />
              <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-control)' }}>{statusText}</span>
              {fileName && <span style={{ marginLeft: 'auto', fontSize: 'var(--text-label)', color: 'var(--text-3)', maxWidth: 220, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{fileName}</span>}
            </div>
          )}

          {/* review：纪要 + 需求卡 */}
          {phase === 'review' && (
            <>
              <div style={{ marginBottom: 14 }}>
                {label('会议标题')}
                <input value={title} onChange={e => setTitle(e.target.value)}
                  style={{ width: '100%', boxSizing: 'border-box', background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 9, padding: '9px 11px', color: 'var(--text)', fontSize: 'var(--text-body)', fontFamily: 'var(--font-sans)', outline: 'none' }} />
              </div>
              <div style={{ marginBottom: 16 }}>
                {label('会议纪要（可编辑，将存入 .autoforge/docs/meetings/）')}
                <textarea value={summary} onChange={e => setSummary(e.target.value)} rows={7}
                  style={{ width: '100%', boxSizing: 'border-box', background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 9, padding: '10px 12px', color: 'var(--text)', fontSize: 'var(--text-body)', fontFamily: 'var(--font-sans)', resize: 'vertical', outline: 'none', lineHeight: 1.6 }} />
              </div>

              <div style={{ display: 'flex', alignItems: 'center', marginBottom: 8 }}>
                {label(`拆解出的需求 · 已选 ${selectedCount}/${reqs.length}`)}
              </div>
              {reqs.length === 0 && (
                <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', padding: '8px 0 4px' }}>AI 未从本次会议中识别到可执行需求。</div>
              )}
              <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
                {reqs.map((r, i) => (
                  <div key={i} style={{
                    border: '1px solid var(--border)', borderRadius: 12, padding: '11px 12px',
                    background: r.selected ? 'var(--bg-3)' : 'transparent', opacity: r.selected ? 1 : 0.55,
                  }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
                      <input type="checkbox" checked={r.selected} onChange={e => patchReq(i, { selected: e.target.checked })}
                        style={{ accentColor: 'var(--ember)', width: 15, height: 15, flexShrink: 0 }} />
                      <input value={r.title} onChange={e => patchReq(i, { title: e.target.value })}
                        placeholder="需求标题"
                        style={{ flex: 1, minWidth: 0, background: 'transparent', border: 'none', borderBottom: '1px solid var(--border)', color: 'var(--text)', fontSize: 'var(--text-body)', fontWeight: 600, fontFamily: 'var(--font-sans)', outline: 'none', padding: '2px 0' }} />
                    </div>
                    <div style={{ display: 'flex', gap: 8, marginBottom: 8 }}>
                      <div style={{ width: 150 }}>
                        <Select value={r.category} onChange={v => patchReq(i, { category: v })} options={CATEGORY_OPTS} />
                      </div>
                      <div style={{ width: 140 }}>
                        <Select value={r.severity} onChange={v => patchReq(i, { severity: v })} options={SEVERITY_OPTS} />
                      </div>
                    </div>
                    <textarea value={r.description} onChange={e => patchReq(i, { description: e.target.value })} rows={2}
                      placeholder="需求描述"
                      style={{ width: '100%', boxSizing: 'border-box', background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 8, padding: '7px 9px', color: 'var(--text-2)', fontSize: 'var(--text-control)', fontFamily: 'var(--font-sans)', resize: 'vertical', outline: 'none', lineHeight: 1.55 }} />
                  </div>
                ))}
              </div>
            </>
          )}

          {/* done */}
          {phase === 'done' && (
            <div style={{ padding: '12px 4px' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, color: 'var(--green)', fontWeight: 600 }}>
                <Icon name="check" size={16} />已完成
              </div>
              <div style={{ marginTop: 10, fontSize: 'var(--text-control)', color: 'var(--text-2)', lineHeight: 1.7 }}>
                · {submittedCount} 条需求已入「待整理池」<br />
                · 纪要已保存到 <code style={{ fontFamily: 'var(--font-mono)', color: 'var(--text)' }}>{savedPath}</code>
              </div>
            </div>
          )}

          {err && (
            <div style={{ marginTop: 12, fontSize: 'var(--text-label)', color: 'var(--red)', lineHeight: 1.5 }}>{err}</div>
          )}
        </div>

        {/* 底部操作 */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '14px 20px', borderTop: '1px solid var(--border)' }}>
          {phase === 'review' && (
            <button className="btn btn-sm btn-ghost" onClick={() => { setPhase('idle'); setReqs([]); setSummary(''); setTranscript(''); setFileName(''); }}>
              <Icon name="refresh" size={13} />重新上传
            </button>
          )}
          <div style={{ flex: 1 }} />
          {phase === 'done' ? (
            <button className="btn btn-primary" onClick={onClose}><Icon name="check" size={14} />完成</button>
          ) : phase === 'review' ? (
            <button className="btn btn-primary" onClick={confirm} disabled={busy || (selectedCount === 0)}>
              <Icon name="send" size={14} />存纪要并入池 {selectedCount > 0 ? `(${selectedCount})` : ''}
            </button>
          ) : (
            <button className="btn btn-sm btn-ghost" onClick={onClose} disabled={busy}>取消</button>
          )}
        </div>
      </div>
    </div>,
    document.querySelector('.os-window') ?? document.body,
  );
}
