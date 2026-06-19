import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { asrRealtimeStart, asrRealtimeFeed, asrRealtimeStop } from '../services';

// Float32 PCM（任意采样率）→ 16kHz 单声道 Int16（DashScope 要求）。
function downsampleToInt16(buffer: Float32Array, inRate: number, outRate = 16000): Int16Array {
  const ratio = inRate > outRate ? inRate / outRate : 1;
  const newLen = Math.floor(buffer.length / ratio);
  const out = new Int16Array(newLen);
  for (let i = 0; i < newLen; i++) {
    const s = Math.max(-1, Math.min(1, buffer[Math.floor(i * ratio)] || 0));
    out[i] = s < 0 ? s * 0x8000 : s * 0x7fff;
  }
  return out;
}

function int16ToBase64(int16: Int16Array): string {
  const bytes = new Uint8Array(int16.buffer, int16.byteOffset, int16.byteLength);
  let bin = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    bin += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + chunk)) as unknown as number[]);
  }
  return btoa(bin);
}

/**
 * 实时语音识别会话：采集麦克风 PCM → 16kHz 分帧 → 经后端代理流式发往阿里 DashScope，
 * 识别结果经 AutoForge://event 的 asr_result 事件回调（增量 + 整句）。
 */
export class RealtimeAsr {
  private sessionId = '';
  private ctx: AudioContext | null = null;
  private processor: ScriptProcessorNode | null = null;
  private source: MediaStreamAudioSourceNode | null = null;
  private stream: MediaStream | null = null;
  private unlisten: UnlistenFn | null = null;

  async start(onResult: (text: string, isFinal: boolean) => void): Promise<void> {
    const md = navigator.mediaDevices;
    if (!md?.getUserMedia) throw new Error('当前环境不支持麦克风');
    // 并行：麦克风授权与后端建链（DashScope WS 握手）同时进行，缩短启动等待。
    const sessP = asrRealtimeStart();
    try {
      this.stream = await md.getUserMedia({ audio: true });
    } catch (e) {
      // 麦克风失败：回收可能已建立的后端会话，避免悬挂。
      void sessP.then((sid) => { if (sid) void asrRealtimeStop(sid); }).catch(() => {});
      throw e;
    }
    this.sessionId = await sessP;

    this.unlisten = await listen('AutoForge://event', (e) => {
      const p = e.payload as { type?: string; session_id?: string; text?: string; is_final?: boolean };
      if (p?.type === 'asr_result' && p.session_id === this.sessionId) {
        onResult(p.text ?? '', Boolean(p.is_final));
      }
    });

    const ctx = new AudioContext();
    this.ctx = ctx;
    const source = ctx.createMediaStreamSource(this.stream);
    this.source = source;
    const processor = ctx.createScriptProcessor(4096, 1, 1);
    this.processor = processor;
    const inRate = ctx.sampleRate;
    processor.onaudioprocess = (ev) => {
      if (!this.sessionId) return;
      const pcm = downsampleToInt16(ev.inputBuffer.getChannelData(0), inRate);
      if (pcm.length) void asrRealtimeFeed(this.sessionId, int16ToBase64(pcm)).catch(() => {});
    };
    source.connect(processor);
    processor.connect(ctx.destination); // 不写 outputBuffer → 静音，仅驱动 onaudioprocess
  }

  async stop(): Promise<void> {
    const sid = this.sessionId;
    this.sessionId = '';
    try { this.processor?.disconnect(); } catch { /* ignore */ }
    try { this.source?.disconnect(); } catch { /* ignore */ }
    try { await this.ctx?.close(); } catch { /* ignore */ }
    this.stream?.getTracks().forEach((t) => t.stop());
    this.processor = null; this.source = null; this.ctx = null; this.stream = null;
    if (this.unlisten) { this.unlisten(); this.unlisten = null; }
    if (sid) { try { await asrRealtimeStop(sid); } catch { /* ignore */ } }
  }
}
