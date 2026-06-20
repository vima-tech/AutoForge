// 长音频分段：把任意格式的会议录音文件解码为 16kHz 单声道 16bit PCM（小端），再按固定
// 时长切成多段（base64）。逐段走后端 `transcribe_recording_segment` —— 与实时麦克风同一条
// 阿里百炼 DashScope 实时 WS 链路，所有语音识别收敛到一种方式。浏览器的 decodeAudioData
// 已覆盖 mp3/m4a/webm/wav/ogg 等容器，故无需在后端引入音频解码依赖。

const TARGET_RATE = 16000;
// 每段时长（秒）。单个实时任务有界，避免超长；240s × 16k × 2B ≈ 7.7MB，远低于 25MB IPC 上限。
const SEGMENT_SEC = 240;

// 任意采样率 Float32 → 16kHz Int16（线性抽取，与 realtimeAsr 一致）。
function downsampleToInt16(buffer: Float32Array, inRate: number, outRate = TARGET_RATE): Int16Array {
  const ratio = inRate > outRate ? inRate / outRate : 1;
  const newLen = Math.floor(buffer.length / ratio);
  const out = new Int16Array(newLen);
  for (let i = 0; i < newLen; i++) {
    const s = Math.max(-1, Math.min(1, buffer[Math.floor(i * ratio)] || 0));
    out[i] = s < 0 ? s * 0x8000 : s * 0x7fff;
  }
  return out;
}

// 多声道下混为单声道。
function mixToMono(audio: AudioBuffer): Float32Array {
  if (audio.numberOfChannels <= 1) return audio.getChannelData(0);
  const len = audio.length;
  const out = new Float32Array(len);
  for (let c = 0; c < audio.numberOfChannels; c++) {
    const ch = audio.getChannelData(c);
    for (let i = 0; i < len; i++) out[i] += ch[i];
  }
  for (let i = 0; i < len; i++) out[i] /= audio.numberOfChannels;
  return out;
}

// Int16 PCM 原始字节（小端，桌面端均为小端，与实时 feed 一致）→ base64。
function int16ToBase64(int16: Int16Array): string {
  const bytes = new Uint8Array(int16.buffer, int16.byteOffset, int16.byteLength);
  let bin = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    bin += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + chunk)) as unknown as number[]);
  }
  return btoa(bin);
}

export interface AudioSegmentsInfo {
  /** 段总数（即需要的转写请求数）。 */
  total: number;
  /** 音频总时长（秒）。 */
  durationSec: number;
}

/**
 * 把音频文件解码 + 切成多段 16kHz 单声道 PCM（base64，按时序）。
 * @param onDecoded 解码完成、开始切段前回调（拿到总时长/段数，便于显示进度上限）。
 */
export async function fileToPcmSegments(
  file: File,
  onDecoded?: (info: AudioSegmentsInfo) => void,
): Promise<string[]> {
  const AC: typeof AudioContext =
    window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
  if (!AC) throw new Error('当前环境不支持音频解码');
  const ctx = new AC();
  let audio: AudioBuffer;
  try {
    audio = await ctx.decodeAudioData(await file.arrayBuffer());
  } catch {
    throw new Error('音频解码失败：文件可能损坏或格式不受支持');
  } finally {
    void ctx.close();
  }

  const mono = mixToMono(audio);
  const pcm16 = downsampleToInt16(mono, audio.sampleRate, TARGET_RATE);
  const samplesPerSeg = TARGET_RATE * SEGMENT_SEC;
  const total = Math.max(1, Math.ceil(pcm16.length / samplesPerSeg));
  onDecoded?.({ total, durationSec: audio.duration });

  const segments: string[] = [];
  for (let i = 0; i < total; i++) {
    const slice = pcm16.subarray(i * samplesPerSeg, Math.min((i + 1) * samplesPerSeg, pcm16.length));
    if (slice.length) segments.push(int16ToBase64(slice));
  }
  return segments;
}
