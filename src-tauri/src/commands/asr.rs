//! 语音转写命令：薄包装（取 state → 调纯 async fn → 返回）。
//!
//! 前端用 MediaRecorder 录音得到音频 blob，base64 经 IPC 传入；后端解码 → 调
//! `agents::asr::transcribe` → 转写文本过 `has_obvious_injection` 后返回。
//! 密钥不出 webview，留在后端配置里。

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::core::asr_realtime::AsrCtl;
use crate::state::AppState;

/// 单条音频上限 25MB（与 OpenAI 一致），防止超大 base64 经 IPC。
const MAX_AUDIO_BYTES: usize = 25 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct TranscribeResult {
    pub text: String,
}

#[tauri::command]
pub async fn transcribe_audio(
    audio_base64: String,
    mime: Option<String>,
    state: State<'_, AppState>,
) -> Result<TranscribeResult, String> {
    let bytes = B64
        .decode(audio_base64.trim())
        .map_err(|e| format!("音频 base64 解码失败：{}", e))?;
    if bytes.is_empty() {
        return Err("音频为空".to_string());
    }
    if bytes.len() > MAX_AUDIO_BYTES {
        return Err("音频超过 25MB 上限，请缩短录音".to_string());
    }

    let mime = mime.unwrap_or_else(|| "audio/webm".to_string());
    let (filename, mime) = normalize_audio_kind(&mime);

    let cfg = crate::agents::asr::AsrConfig::load(&state.db).await;
    let text = crate::agents::asr::transcribe(&cfg, bytes, filename, mime)
        .await
        .map_err(|e| e.to_string())?;

    if crate::core::security::has_obvious_injection(&text) {
        return Err("转写文本包含可疑内容，已拒绝".to_string());
    }

    Ok(TranscribeResult { text })
}

// ── 实时语音识别（阿里 DashScope 流式）────────────────────────────────────────

/// 开启一个实时识别会话，返回 session_id。结果经 `AutoForge://event` 的 AsrResult 推送。
#[tauri::command]
pub async fn asr_realtime_start(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let session_id = uuid::Uuid::new_v4().to_string();
    let tx = crate::core::asr_realtime::start_session(&state.db, &app, session_id.clone()).await?;
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

/// 由前端 MIME 映射出 OpenAI 可识别的文件名与归一 MIME。
fn normalize_audio_kind(mime: &str) -> (&'static str, &'static str) {
    let m = mime.split(';').next().unwrap_or(mime).trim();
    match m {
        "audio/webm" => ("audio.webm", "audio/webm"),
        "audio/ogg" | "audio/opus" => ("audio.ogg", "audio/ogg"),
        "audio/wav" | "audio/x-wav" | "audio/wave" => ("audio.wav", "audio/wav"),
        "audio/mpeg" | "audio/mp3" => ("audio.mp3", "audio/mpeg"),
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => ("audio.mp4", "audio/mp4"),
        "audio/flac" => ("audio.flac", "audio/flac"),
        _ => ("audio.webm", "audio/webm"),
    }
}
