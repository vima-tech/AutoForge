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
  private stopped = false;
  // 后端 WS 握手完成前先把音频帧缓存于此，握手就绪后一次性冲刷，避免丢失开头语音。
  private pending: string[] = [];

  /**
   * @param onResult 识别结果回调（增量/整句）。
   * @param onLive   麦克风已授权、采集管线就绪时触发（**早于**后端 DashScope 握手完成）——
   *                 供 UI 立即进入「聆听中」，消除点击后的等待空窗。
   */
  async start(
    onResult: (text: string, isFinal: boolean) => void,
    onLive?: () => void,
  ): Promise<void> {
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
    if (this.stopped) { // start 期间已被取消：释放刚拿到的麦克风与后端会话。
      this.stream.getTracks().forEach((t) => t.stop());
      void sessP.then((sid) => { if (sid) void asrRealtimeStop(sid); }).catch(() => {});
      return;
    }

    // 麦克风已就绪：立刻搭建采集管线并开始收音（握手未完成前帧先入 pending 缓冲），
    // 让 UI 立即「聆听中」，同时保证后端连上后开头语音不丢。
    const ctx = new AudioContext();
    this.ctx = ctx;
    const source = ctx.createMediaStreamSource(this.stream);
    this.source = source;
    const processor = ctx.createScriptProcessor(4096, 1, 1);
    this.processor = processor;
    const inRate = ctx.sampleRate;
    processor.onaudioprocess = (ev) => {
      const pcm = downsampleToInt16(ev.inputBuffer.getChannelData(0), inRate);
      if (!pcm.length) return;
      const b64 = int16ToBase64(pcm);
      if (this.sessionId) void asrRealtimeFeed(this.sessionId, b64).catch(() => {});
      else this.pending.push(b64);
    };
    source.connect(processor);
    processor.connect(ctx.destination); // 不写 outputBuffer → 静音，仅驱动 onaudioprocess
    onLive?.();

    const sessionId = await sessP;

    const unlisten = await listen('AutoForge://event', (e) => {
      const p = e.payload as { type?: string; session_id?: string; text?: string; is_final?: boolean };
      if (p?.type === 'asr_result' && p.session_id === sessionId) {
        onResult(p.text ?? '', Boolean(p.is_final));
      }
    });
    // 握手期间被 stop()：监听器在此分支自行回收（彼时 stop 已无法看到它），并收束后端会话。
    if (this.stopped) { unlisten(); void asrRealtimeStop(sessionId).catch(() => {}); return; }
    this.unlisten = unlisten;
    this.sessionId = sessionId;
    // 冲刷握手期间缓冲的音频帧。
    const buffered = this.pending;
    this.pending = [];
    for (const b64 of buffered) void asrRealtimeFeed(sessionId, b64).catch(() => {});
  }

  async stop(): Promise<void> {
    this.stopped = true; // 让仍在 start() 中的握手在完成后自行清理。
    const sid = this.sessionId;
    this.sessionId = '';
    this.pending = [];
    try { this.processor?.disconnect(); } catch { /* ignore */ }
    try { this.source?.disconnect(); } catch { /* ignore */ }
    try { await this.ctx?.close(); } catch { /* ignore */ }
    this.stream?.getTracks().forEach((t) => t.stop());
    this.processor = null; this.source = null; this.ctx = null; this.stream = null;
    if (this.unlisten) { this.unlisten(); this.unlisten = null; }
    if (sid) { try { await asrRealtimeStop(sid); } catch { /* ignore */ } }
  }
}
