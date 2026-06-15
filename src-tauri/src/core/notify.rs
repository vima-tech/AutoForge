//! M11 — notification hub (design §13).
//!
//! Dispatches factory events to configured external channels (Slack / 企业微信 /
//! generic webhook) over HTTP. All sends are best-effort: a missing or failing
//! channel never blocks the pipeline. Email is reachable via a generic webhook
//! bridge (no SMTP dependency is linked).

use crate::db::Db;
use serde_json::{json, Value};
use std::time::Duration;

/// Fire a notification for `event_kind` to every enabled channel that subscribes
/// to it (empty `events` filter = all events). Best-effort, fire-and-forget safe.
pub async fn dispatch(db: &Db, event_kind: &str, title: &str, body: &str) {
    let channels = sqlx::query_as::<_, (String, String, String)>(
        "SELECT kind, target, events FROM notify_channels WHERE enabled=1",
    )
    .fetch_all(db)
    .await
    .unwrap_or_default();
    if channels.is_empty() {
        return;
    }
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
    else {
        return;
    };
    for (kind, target, events) in channels {
        if target.trim().is_empty() {
            continue;
        }
        if !events.trim().is_empty()
            && !events.split(',').any(|e| e.trim() == event_kind)
        {
            continue;
        }
        if kind == "email" {
            let _ = send_email(
                &target,
                &format!("AutoForge · {event_kind}"),
                &format!("{title}\n{body}"),
            )
            .await;
            continue;
        }
        let payload = build_payload(&kind, event_kind, title, body);
        let _ = client.post(&target).json(&payload).send().await;
    }
}

/// SMTP email delivery. `target` format: `smtp(s)://user:pass@host:port?from=a@b&to=c@d`.
async fn send_email(target: &str, subject: &str, body: &str) -> Result<(), String> {
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

    let (relay, query) = target.split_once('?').unwrap_or((target, ""));
    let mut from = String::new();
    let mut to = String::new();
    for kv in query.split('&') {
        if let Some(v) = kv.strip_prefix("from=") {
            from = urldecode(v);
        } else if let Some(v) = kv.strip_prefix("to=") {
            to = urldecode(v);
        }
    }
    if from.is_empty() || to.is_empty() {
        return Err("email 目标需包含 ?from=...&to=...".to_string());
    }
    let email = Message::builder()
        .from(from.parse().map_err(|e| format!("from 解析失败: {e}"))?)
        .to(to.parse().map_err(|e| format!("to 解析失败: {e}"))?)
        .subject(subject)
        .body(body.to_string())
        .map_err(|e| e.to_string())?;
    let mailer = AsyncSmtpTransport::<Tokio1Executor>::from_url(relay)
        .map_err(|e| e.to_string())?
        .build();
    mailer.send(email).await.map_err(|e| e.to_string())?;
    Ok(())
}

fn urldecode(s: &str) -> String {
    s.replace("%40", "@").replace("%20", " ").replace('+', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    async fn mem_pool(kind: &str, target: &str, events: &str) -> Db {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE notify_channels (id TEXT PRIMARY KEY, name TEXT, kind TEXT, target TEXT, events TEXT, enabled INTEGER)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO notify_channels (id,name,kind,target,events,enabled) VALUES ('1','t',?,?,?,1)",
        )
        .bind(kind)
        .bind(target)
        .bind(events)
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn capture_server() -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
        let received = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let r2 = received.clone();
        let app = axum::Router::new().route(
            "/hook",
            axum::routing::post(move |axum::Json(v): axum::Json<serde_json::Value>| {
                let r = r2.clone();
                async move {
                    r.lock().await.push(v);
                    axum::http::StatusCode::OK
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        (format!("http://{addr}/hook"), received)
    }

    #[tokio::test]
    async fn dispatch_delivers_over_real_http() {
        let (url, received) = capture_server().await;
        let pool = mem_pool("webhook", &url, "").await;
        dispatch(&pool, "review_needed", "标题", "正文").await;
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        let got = received.lock().await;
        assert_eq!(got.len(), 1, "应收到 1 条 HTTP 投递");
        assert_eq!(got[0]["title"], "标题");
        assert_eq!(got[0]["event"], "review_needed");
    }

    #[tokio::test]
    async fn dispatch_respects_event_filter() {
        let (url, received) = capture_server().await;
        // channel only subscribes to cr_merged; a review_needed event must NOT deliver
        let pool = mem_pool("webhook", &url, "cr_merged").await;
        dispatch(&pool, "review_needed", "x", "y").await;
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        assert_eq!(received.lock().await.len(), 0);
    }

    #[test]
    fn slack_and_wecom_payload_shapes() {
        let slack = build_payload("slack", "cr_merged", "t", "b");
        assert!(slack.get("text").is_some());
        let wecom = build_payload("wecom", "cr_merged", "t", "b");
        assert_eq!(wecom["msgtype"], "text");
        assert!(wecom["text"]["content"].is_string());
    }
}

fn build_payload(kind: &str, event_kind: &str, title: &str, body: &str) -> Value {
    let text = format!("【AutoForge · {event_kind}】{title}\n{body}");
    match kind {
        "slack" => json!({ "text": text }),
        "wecom" => json!({ "msgtype": "text", "text": { "content": text } }),
        _ => json!({ "event": event_kind, "title": title, "body": body }),
    }
}

/// Send a one-off test message to a channel target (used by the settings UI).
pub async fn send_test(kind: &str, target: &str) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("目标地址为空".to_string());
    }
    if kind == "email" {
        return send_email(target, "AutoForge 通知测试", "AutoForge 通知通道连通性测试").await;
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let payload = build_payload(kind, "test", "通知测试", "AutoForge 通知通道连通性测试");
    let resp = client
        .post(target)
        .json(&payload)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", resp.status().as_u16()))
    }
}
