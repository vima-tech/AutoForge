//! 实时语音识别（ASR）：后端 WebSocket 代理对接**阿里 DashScope Paraformer 实时识别**。
//!
//! 为什么走后端代理：浏览器 WebSocket 无法设置鉴权 header（DashScope 需 `Authorization`
//! header），且 API Key 按规范不得下放前端。故由 Rust 持密钥开 WS，前端经 IPC 推送
//! PCM 音频帧，识别结果经 `event::emit(AsrResult)` 回推前端实时显示。
//!
//! 协议（paraformer-realtime-v2，PCM 16k 单声道）：connect → run-task → 流式发二进制音频
//! → result-generated(增量/整句) → finish-task → task-finished。
//!
//! Tauri 耦合仅限 `AppHandle` 用于事件发射（CLAUDE.md 允许的唯一例外）。

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

use crate::core::event::{self, AppEvent};
use crate::db::Db;

const DASHSCOPE_WS: &str = "wss://dashscope.aliyuncs.com/api-ws/v1/inference/";

/// 会话控制消息：音频帧或结束。
pub enum AsrCtl {
    Audio(Vec<u8>),
    Finish,
}

async fn get_setting(db: &Db, key: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key=?")
        .bind(key)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
}

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

/// **全局唯一的百炼 ASR 接入点**：解析 API Key + 模型，与 DashScope 建链，返回
/// `(ws, run-task JSON, task_id)`。实时麦克风（[`start_session`]）与会议录音上传
/// （[`transcribe_segment`]）共用此入口——所有语音识别都收敛到这一条链路。
async fn open_ws(db: &Db) -> Result<(WsStream, Value, String), String> {
    let api_key = crate::core::secrets::decrypt(
        &get_setting(db, "asr.api_key").await.unwrap_or_default(),
    )
    .unwrap_or_default();
    if api_key.trim().is_empty() {
        return Err("语音识别未配置 API Key（设置 → 语音录入，填阿里百炼 DashScope API Key）".into());
    }
    let model = get_setting(db, "asr.model")
        .await
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "paraformer-realtime-v2".to_string());

    // 建链（带鉴权 header）。
    let mut req = DASHSCOPE_WS
        .into_client_request()
        .map_err(|e| format!("WS 请求构造失败：{}", e))?;
    {
        let h = req.headers_mut();
        h.insert(
            "Authorization",
            format!("bearer {}", api_key.trim())
                .parse()
                .map_err(|_| "API Key 含非法字符".to_string())?,
        );
        h.insert("X-DashScope-DataInspection", "enable".parse().unwrap());
    }
    let (ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .map_err(|e| format!("连接 DashScope 失败：{}", e))?;

    let task_id = uuid::Uuid::new_v4().simple().to_string();
    let run_task = json!({
        "header": { "action": "run-task", "task_id": task_id, "streaming": "duplex" },
        "payload": {
            "task_group": "audio", "task": "asr", "function": "recognition",
            "model": model,
            "parameters": { "format": "pcm", "sample_rate": 16000 },
            "input": {}
        }
    });
    Ok((ws, run_task, task_id))
}

/// 打开一个实时识别会话，返回向其推送音频/结束信号的发送端。
/// 后台任务负责：run-task 握手 → 转发音频 → 解析结果发事件 → finish。
pub async fn start_session(
    db: &Db,
    app: &AppHandle,
    session_id: String,
) -> Result<mpsc::UnboundedSender<AsrCtl>, String> {
    let (ws, run_task, task_id) = open_ws(db).await?;
    let (tx, rx) = mpsc::unbounded_channel::<AsrCtl>();
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(e) = drive(ws, rx, run_task, task_id, &app, &session_id).await {
            tracing::warn!("[asr] 会话 {} 结束：{}", session_id, e);
        }
    });

    Ok(tx)
}

/// 收集模式：把**一整段** 16kHz 单声道 16bit PCM（小端）经同一条 DashScope 实时 WS
/// 转写为完整文本（用于会议录音上传）。与实时麦克风共用 [`open_ws`]，只是把结果由
/// 「发事件」改成「累计整句后返回」。调用方应把长录音切成有界时长的段分别调用，避免单个
/// 实时任务过长。
pub async fn transcribe_segment(db: &Db, pcm: Vec<u8>) -> Result<String, String> {
    let (ws, run_task, task_id) = open_ws(db).await?;
    let (mut write, mut read) = ws.split();
    write
        .send(Message::Text(run_task.to_string()))
        .await
        .map_err(|e| format!("run-task 发送失败：{}", e))?;

    // 音频分 ~8KB 二进制帧发送（结果消息很小，先写后读不会阻塞）。
    for frame in pcm.chunks(8192) {
        write
            .send(Message::Binary(frame.to_vec()))
            .await
            .map_err(|e| format!("音频发送失败：{}", e))?;
    }
    let fin = json!({
        "header": { "action": "finish-task", "task_id": task_id, "streaming": "duplex" },
        "payload": { "input": {} }
    });
    let _ = write.send(Message::Text(fin.to_string())).await;

    // 读取直至 task-finished，累计每条「整句」。
    let mut out = String::new();
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(t)) => {
                if collect_sentence(&t, &mut out) {
                    break;
                }
            }
            Ok(Message::Close(_)) => break,
            Err(e) => return Err(format!("WS 读取错误：{}", e)),
            _ => {}
        }
    }
    Ok(out.trim().to_string())
}

/// 解析一条服务端事件，把「整句」追加进 `out`；返回 true 表示任务终止。
fn collect_sentence(text: &str, out: &mut String) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return false };
    match v["header"]["event"].as_str() {
        Some("result-generated") => {
            let sentence = &v["payload"]["output"]["sentence"];
            if sentence["sentence_end"].as_bool().unwrap_or(false) {
                let t = sentence["text"].as_str().unwrap_or("").trim();
                if !t.is_empty() {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(t);
                }
            }
            false
        }
        Some("task-finished") | Some("task-failed") => true,
        _ => false,
    }
}

/// 会话主循环：拆分读写两半，select 转发音频 / 解析结果。
async fn drive(
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    mut rx: mpsc::UnboundedReceiver<AsrCtl>,
    run_task: Value,
    task_id: String,
    app: &AppHandle,
    session_id: &str,
) -> Result<()> {
    let (mut write, mut read) = ws.split();
    write
        .send(Message::Text(run_task.to_string()))
        .await
        .map_err(|e| anyhow!("发送 run-task 失败：{}", e))?;

    let mut finishing = false;
    loop {
        tokio::select! {
            ctl = rx.recv(), if !finishing => match ctl {
                Some(AsrCtl::Audio(bytes)) => {
                    if write.send(Message::Binary(bytes)).await.is_err() { break; }
                }
                Some(AsrCtl::Finish) | None => {
                    let fin = json!({
                        "header": { "action": "finish-task", "task_id": task_id, "streaming": "duplex" },
                        "payload": { "input": {} }
                    });
                    let _ = write.send(Message::Text(fin.to_string())).await;
                    finishing = true;
                }
            },
            msg = read.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    if handle_event(&t, app, session_id) { break; }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(e)) => return Err(anyhow!("WS 读取错误：{}", e)),
                _ => {}
            }
        }
    }
    Ok(())
}

/// 解析一条服务端 JSON 事件；返回 true 表示会话应终止。
fn handle_event(text: &str, app: &AppHandle, session_id: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return false };
    match v["header"]["event"].as_str() {
        Some("result-generated") => {
            let sentence = &v["payload"]["output"]["sentence"];
            let out = sentence["text"].as_str().unwrap_or("").trim().to_string();
            if !out.is_empty() {
                let is_final = sentence["sentence_end"].as_bool().unwrap_or(false);
                event::emit(
                    app,
                    AppEvent::AsrResult {
                        session_id: session_id.to_string(),
                        text: out,
                        is_final,
                    },
                );
            }
            false
        }
        Some("task-finished") | Some("task-failed") => true,
        _ => false,
    }
}
