//! 语音识别命令：薄包装（取 state → 调纯 async fn → 返回）。
//!
//! 全部语音识别（实时麦克风 + 会议录音上传）统一收敛到**阿里百炼 DashScope 实时 WS**
//! （`core/asr_realtime.rs`）：
//!   - `asr_realtime_*`：实时麦克风边说边出字（事件回推）；
//!   - `transcribe_recording_segment`：会议录音整段转写（收集模式，返回完整文本）。
//!
//! 密钥不出 webview，留在后端配置里。转写文本是不可信外部输入，过 `has_obvious_injection`。

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use tauri::{AppHandle, State};

use crate::core::asr_realtime::AsrCtl;
use crate::state::AppState;

/// 单段音频上限 25MB，防止超大 base64 经 IPC（前端已按时长切段，单段远小于此）。
const MAX_SEGMENT_BYTES: usize = 25 * 1024 * 1024;

/// 转写一整段 16kHz/单声道/16bit 小端 PCM（base64），经 DashScope 实时 WS 收集完整文本。
/// 会议录音上传的长音频由前端切成多段、逐段调用本命令后拼接。
#[tauri::command]
pub async fn transcribe_recording_segment(
    pcm_base64: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let bytes = B64
        .decode(pcm_base64.trim())
        .map_err(|e| format!("音频 base64 解码失败：{}", e))?;
    if bytes.is_empty() {
        return Err("音频段为空".to_string());
    }
    if bytes.len() > MAX_SEGMENT_BYTES {
        return Err("音频段过大，请减小分段时长".to_string());
    }

    let text = crate::core::asr_realtime::transcribe_segment(&state.db, bytes).await?;

    if !text.is_empty() && crate::core::security::has_obvious_injection(&text) {
        return Err("转写文本包含可疑内容，已拒绝".to_string());
    }
    Ok(text)
}

/// 原始录音文件上限 100MB（前端 WebView 解码失败时的后端兜底入口；压缩音频远小于此）。
const MAX_FILE_BYTES: usize = 100 * 1024 * 1024;
/// 后端解码后按 240s 分段送 WS（与前端分段一致，避免单个实时任务过长）。
const SEGMENT_SAMPLES: usize = 240 * 16000;

/// 兜底入口：当浏览器 WebView 解不出某音频格式时，前端把**原始文件字节**(base64) 交后端，
/// 用 symphonia 内置解码（mp3/m4a-aac/wav/flac/ogg-vorbis）→ 16k 单声道 PCM → 分段经
/// DashScope 实时 WS 转写 → 拼接完整文本。`mime` 作格式探测提示。
#[tauri::command]
pub async fn transcribe_recording_file(
    file_base64: String,
    mime: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let bytes = B64
        .decode(file_base64.trim())
        .map_err(|e| format!("文件 base64 解码失败：{}", e))?;
    if bytes.is_empty() {
        return Err("音频文件为空".to_string());
    }
    if bytes.len() > MAX_FILE_BYTES {
        return Err("音频文件超过 100MB，请压缩或分割后再上传".to_string());
    }

    // 解码 CPU 密集，放 blocking 线程。
    let pcm = tokio::task::spawn_blocking(move || {
        crate::core::audio_decode::decode_to_pcm16k_mono(&bytes, mime.as_deref())
    })
    .await
    .map_err(|e| format!("解码任务失败：{}", e))??;

    let mut out = String::new();
    for seg in pcm.chunks(SEGMENT_SAMPLES) {
        let frame = crate::core::audio_decode::i16_to_le_bytes(seg);
        let text = crate::core::asr_realtime::transcribe_segment(&state.db, frame).await?;
        if !text.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&text);
        }
    }

    if !out.is_empty() && crate::core::security::has_obvious_injection(&out) {
        return Err("转写文本包含可疑内容，已拒绝".to_string());
    }
    Ok(out)
}

// ── 实时语音识别（阿里 DashScope 流式）────────────────────────────────────────

/// 开启一个实时识别会话，返回 session_id。结果经 `autoforge://event` 的 AsrResult 推送。
#[tauri::command]
pub async fn asr_realtime_start(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let session_id = uuid::Uuid::new_v4().to_string();
    let tx = crate::core::asr_realtime::start_session(
        &state.db,
        std::sync::Arc::new(app.clone()),
        session_id.clone(),
    )
    .await?;
    state.asr_sessions.lock().await.insert(session_id.clone(), tx);
    Ok(session_id)
}

/// 向实时会话推送一帧 PCM 音频（base64，16kHz 单声道 16bit）。
#[tauri::command]
pub async fn asr_realtime_feed(
    session_id: String,
    audio_base64: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let bytes = B64
        .decode(audio_base64.trim())
        .map_err(|e| format!("音频解码失败：{}", e))?;
    if let Some(tx) = state.asr_sessions.lock().await.get(&session_id) {
        let _ = tx.send(AsrCtl::Audio(bytes));
    }
    Ok(())
}

/// 结束实时会话（发送 finish-task 并清理）。
#[tauri::command]
pub async fn asr_realtime_stop(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if let Some(tx) = state.asr_sessions.lock().await.remove(&session_id) {
        let _ = tx.send(AsrCtl::Finish);
    }
    Ok(())
}
