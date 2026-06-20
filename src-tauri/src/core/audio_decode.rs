//! 会议录音内置解码（纯 Rust，零系统依赖）。
//!
//! 主路径在前端用 WebView 的 `decodeAudioData` 解码（覆盖各容器/编码，含 Opus）。当某台机器
//! 的 WebView 缺少解码插件（典型：精简 Linux 缺 mp3/aac 的 GStreamer 插件）解不出时，前端把
//! **原始文件字节**交后端，由本模块用 symphonia 兜底解码——保证 mp3 / m4a-aac / wav / flac /
//! ogg-vorbis 三端一致可用。Opus（ogg/webm 内常见）暂无成熟纯 Rust 解码器，仍依赖 WebView。
//!
//! 输出统一为 **16kHz 单声道 16bit（小端）PCM**，与实时麦克风同一条 DashScope 链路对接。

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

const TARGET_RATE: u32 = 16000;

/// 解码任意（受支持的）音频字节为 16kHz 单声道 i16 PCM。`mime` 仅作格式探测提示（可空）。
/// CPU 密集，调用方应放在 `spawn_blocking` 中执行。
pub fn decode_to_pcm16k_mono(bytes: &[u8], mime: Option<&str>) -> Result<Vec<i16>, String> {
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes.to_vec())), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = mime.and_then(mime_to_ext) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions { enable_gapless: true, ..Default::default() }, &MetadataOptions::default())
        .map_err(|e| format!("无法识别音频格式（可能是 Opus 或不受支持的编码）：{}", e))?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "音频中没有可解码的音轨".to_string())?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("不支持的音频编码：{}", e))?;

    let mut src_rate: u32 = track.codec_params.sample_rate.unwrap_or(TARGET_RATE);
    let mut mono: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(_) => break, // 末尾/容器收尾：已收集到的样本即为结果
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                let spec = *audio_buf.spec();
                src_rate = spec.rate;
                let ch = spec.channels.count().max(1);
                let mut sb = SampleBuffer::<f32>::new(audio_buf.capacity() as u64, spec);
                sb.copy_interleaved_ref(audio_buf);
                for frame in sb.samples().chunks(ch) {
                    let sum: f32 = frame.iter().copied().sum();
                    mono.push(sum / ch as f32);
                }
            }
            Err(SymError::DecodeError(_)) => continue, // 跳过坏帧，继续
            Err(_) => break,
        }
    }

    if mono.is_empty() {
        return Err("解码后音频为空（文件可能损坏或为静音）".to_string());
    }
    Ok(resample_to_16k_i16(&mono, src_rate))
}

/// 线性抽取重采样到 16kHz 并量化为 i16（与前端 downsampleToInt16 一致，语音足够）。
fn resample_to_16k_i16(input: &[f32], in_rate: u32) -> Vec<i16> {
    if input.is_empty() {
        return Vec::new();
    }
    let ratio = if in_rate > TARGET_RATE {
        in_rate as f64 / TARGET_RATE as f64
    } else {
        1.0
    };
    let new_len = (input.len() as f64 / ratio).floor() as usize;
    let mut out = Vec::with_capacity(new_len);
    for i in 0..new_len {
        let src = (i as f64 * ratio) as usize;
        let s = input.get(src).copied().unwrap_or(0.0).clamp(-1.0, 1.0);
        out.push(if s < 0.0 {
            (s * 32768.0) as i16
        } else {
            (s * 32767.0) as i16
        });
    }
    out
}

/// i16 PCM → 小端字节（喂 DashScope 的 binary 帧格式）。
pub fn i16_to_le_bytes(pcm: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pcm.len() * 2);
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

fn mime_to_ext(mime: &str) -> Option<&'static str> {
    match mime.split(';').next().unwrap_or(mime).trim() {
        "audio/mpeg" | "audio/mp3" => Some("mp3"),
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" | "audio/aac" => Some("m4a"),
        "audio/wav" | "audio/x-wav" | "audio/wave" => Some("wav"),
        "audio/flac" | "audio/x-flac" => Some("flac"),
        "audio/ogg" | "audio/x-ogg" => Some("ogg"),
        _ => None,
    }
}
