//! M11 — notification hub (design §13).
//!
//! Dispatches factory events to configured external channels over HTTP. Supported
//! kinds: `slack` / `wecom`（企业微信群机器人）/ `feishu`（飞书自定义机器人，可加签）/
//! `dingtalk`（钉钉自定义机器人，可加签）/ `ntfy` / `clawbot`（微信 OpenClaw bot）/
//! `email`（SMTP）/ `webhook`（通用）。
//!
//! All sends are best-effort: a missing or failing channel never blocks the pipeline.
//! Per-channel credentials (签名 secret / Bearer token) 存于加密的 `secret` 列，
//! 发送前经 `core::secrets::decrypt` 解密；`target` 仍是明文 URL。

use crate::db::Db;
use crate::models::notify::NotifyChannel;
use serde_json::{json, Value};
use std::time::Duration;

/// Fire a notification for `event_kind` to every enabled channel that subscribes
/// to it (empty `events` filter = all events). Best-effort, fire-and-forget safe.
pub async fn dispatch(db: &Db, event_kind: &str, title: &str, body: &str) {
    let channels = sqlx::query_as::<_, (String, String, String, String)>(
        "SELECT kind, target, events, secret FROM notify_channels WHERE enabled=1",
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
    for (kind, target, events, secret) in channels {
        if target.trim().is_empty() {
            continue;
        }
        if !events.trim().is_empty() && !events.split(',').any(|e| e.trim() == event_kind) {
            continue;
        }
        // 解密失败（如缺主密钥）按空 secret 处理，不阻塞其它通道。
        let secret = crate::core::secrets::decrypt(&secret).unwrap_or_default();
        let _ = send_to_channel(&client, &kind, &target, &secret, event_kind, title, body).await;
    }
}

/// Send a one-off test message to a saved channel (used by the settings UI).
/// 按 id 载入通道并解密 secret，再走与 `dispatch` 相同的发送路径。
pub async fn send_test(id: &str, db: &Db) -> Result<(), String> {
    let ch = sqlx::query_as::<_, NotifyChannel>("SELECT * FROM notify_channels WHERE id=?")
        .bind(id)
        .fetch_optional(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "通道不存在".to_string())?;
    if ch.target.trim().is_empty() {
        return Err("目标地址为空".to_string());
    }
    let secret = crate::core::secrets::decrypt(&ch.secret).unwrap_or_default();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    send_to_channel(
        &client,
        &ch.kind,
        &ch.target,
        &secret,
        "test",
        "通知测试",
        "AutoForge 通知通道连通性测试",
    )
    .await
}

/// 按通道类型构造并发出请求（含签名 / 自定义 headers）。
async fn send_to_channel(
    client: &reqwest::Client,
    kind: &str,
    target: &str,
    secret: &str,
    event_kind: &str,
    title: &str,
    body: &str,
) -> Result<(), String> {
    let text = format!("【AutoForge · {event_kind}】{title}\n{body}");
    match kind {
        "email" => {
            return send_email(target, &format!("AutoForge · {event_kind}"), &format!("{title}\n{body}")).await;
        }
        "feishu" => {
            let mut payload = json!({ "msg_type": "text", "content": { "text": text } });
            if !secret.is_empty() {
                let ts = (now_millis() / 1000).to_string();
                let sign = feishu_sign(&ts, secret);
                payload["timestamp"] = json!(ts);
                payload["sign"] = json!(sign);
            }
            return post_json(client, target, &payload).await;
        }
        "dingtalk" => {
            let mut url = target.to_string();
            if !secret.is_empty() {
                let ts = now_millis().to_string();
                let sign = percent_encode(&dingtalk_sign(&ts, secret));
                let sep = if url.contains('?') { '&' } else { '?' };
                url.push_str(&format!("{sep}timestamp={ts}&sign={sign}"));
            }
            let payload = json!({ "msgtype": "text", "text": { "content": text } });
            return post_json(client, &url, &payload).await;
        }
        "ntfy" => {
            let mut req = client
                .post(target)
                .header("X-Title", "AutoForge")
                .header("X-Tags", event_kind)
                .body(format!("{title}\n{body}"));
            if !secret.is_empty() {
                req = req.header("Authorization", format!("Bearer {secret}"));
            }
            return finish(req).await;
        }
        "clawbot" => {
            // target = 绑定时拿到的 baseurl，query 携带 to_user_id / 可选 context_token；secret = bot_token。
            let (base, query) = target.split_once('?').unwrap_or((target, ""));
            let mut to_user_id = String::new();
            let mut context_token = String::new();
            for kv in query.split('&') {
                if let Some(v) = kv.strip_prefix("to_user_id=") {
                    to_user_id = urldecode(v);
                } else if let Some(v) = kv.strip_prefix("context_token=") {
                    context_token = urldecode(v);
                }
            }
            let mut msg = json!({
                "from_user_id": "",
                "to_user_id": to_user_id,
                "client_id": format!("af-{}", now_millis()),
                "message_type": 2,   // MessageType.BOT
                "message_state": 2,  // MessageState.FINISH
                "item_list": [ { "type": 1, "text_item": { "text": text } } ]
            });
            if !context_token.is_empty() {
                msg["context_token"] = json!(context_token);
            }
            let payload = json!({ "msg": msg, "base_info": clawbot_base_info() });
            let url = join_url(base, "ilink/bot/sendmessage");
            let req = clawbot_headers(client.post(&url), Some(secret)).json(&payload);
            return finish(req).await;
        }
        _ => {
            let payload = build_payload(kind, event_kind, title, body);
            return post_json(client, target, &payload).await;
        }
    }
}

async fn post_json(client: &reqwest::Client, url: &str, payload: &Value) -> Result<(), String> {
    finish(client.post(url).json(payload)).await
}

async fn finish(req: reqwest::RequestBuilder) -> Result<(), String> {
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", resp.status().as_u16()))
    }
}

/// 纯 JSON body 形状（不含签名 / headers），便于单测断言。
fn build_payload(kind: &str, event_kind: &str, title: &str, body: &str) -> Value {
    let text = format!("【AutoForge · {event_kind}】{title}\n{body}");
    match kind {
        "slack" => json!({ "text": text }),
        "wecom" => json!({ "msgtype": "text", "text": { "content": text } }),
        "feishu" => json!({ "msg_type": "text", "content": { "text": text } }),
        "dingtalk" => json!({ "msgtype": "text", "text": { "content": text } }),
        _ => json!({ "event": event_kind, "title": title, "body": body }),
    }
}

fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn hmac_sha256_b64(key: &[u8], msg: &[u8]) -> String {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = <Hmac<Sha256>>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(msg);
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// 飞书：sign = base64(HMAC_SHA256(key = "{timestamp}\n{secret}", data = 空))。
fn feishu_sign(timestamp: &str, secret: &str) -> String {
    let key = format!("{timestamp}\n{secret}");
    hmac_sha256_b64(key.as_bytes(), b"")
}

/// 钉钉：sign = base64(HMAC_SHA256(key = secret, data = "{timestamp}\n{secret}"))，再做 URL 编码。
fn dingtalk_sign(timestamp: &str, secret: &str) -> String {
    let data = format!("{timestamp}\n{secret}");
    hmac_sha256_b64(secret.as_bytes(), data.as_bytes())
}

// —— 微信 ClawBot（OpenClaw）协议常量，对齐 Tencent/openclaw-weixin ——
const CLAWBOT_BASE: &str = "https://ilinkai.weixin.qq.com";
const CLAWBOT_APP_ID: &str = "bot";
/// iLink-App-ClientVersion：由 v2.4.3 算 (2<<16)|(4<<8)|3。
const CLAWBOT_CLIENT_VERSION: &str = "132099";
const CLAWBOT_CHANNEL_VERSION: &str = "2.4.3";
const CLAWBOT_BOT_AGENT: &str = "OpenClaw";
const CLAWBOT_BOT_TYPE: &str = "3";

/// X-WECHAT-UIN：随机 u32 → 十进制字符串 → base64（对齐 openclaw randomWechatUin）。
fn wechat_uin() -> String {
    use base64::Engine;
    // 无 rand 依赖，用纳秒时钟混淆出一个伪随机 u32 作为 nonce。
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = (nanos as u32) ^ ((nanos >> 32) as u32).wrapping_mul(2654435761);
    base64::engine::general_purpose::STANDARD.encode(n.to_string().as_bytes())
}

fn clawbot_base_info() -> Value {
    json!({ "channel_version": CLAWBOT_CHANNEL_VERSION, "bot_agent": CLAWBOT_BOT_AGENT })
}

/// 拼接 base + path，避免出现双斜杠。
fn join_url(base: &str, path: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), path.trim_start_matches('/'))
}

/// 给请求加上 ClawBot 全程必带的 headers；`token` 存在时附 Bearer。
fn clawbot_headers(req: reqwest::RequestBuilder, token: Option<&str>) -> reqwest::RequestBuilder {
    let mut req = req
        .header("Content-Type", "application/json")
        .header("AuthorizationType", "ilink_bot_token")
        .header("X-WECHAT-UIN", wechat_uin())
        .header("iLink-App-Id", CLAWBOT_APP_ID)
        .header("iLink-App-ClientVersion", CLAWBOT_CLIENT_VERSION);
    if let Some(t) = token {
        if !t.trim().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", t.trim()));
        }
    }
    req
}

/// 扫码绑定第 1 步：申请二维码。返回轮询用 qrcode、可渲染的 SVG（data URL）、轮询 base。
#[derive(serde::Serialize)]
pub struct ClawbotQrStart {
    pub qrcode: String,
    pub qr_svg: String,
    pub qr_url: String,
    pub base_url: String,
}

pub async fn clawbot_qr_start() -> Result<ClawbotQrStart, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{CLAWBOT_BASE}/ilink/bot/get_bot_qrcode?bot_type={CLAWBOT_BOT_TYPE}");
    let resp = clawbot_headers(client.post(&url), None)
        .json(&json!({ "local_token_list": [] }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("get_bot_qrcode HTTP {}", resp.status().as_u16()));
    }
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    let qrcode = v["qrcode"].as_str().unwrap_or_default().to_string();
    let qr_url = v["qrcode_img_content"].as_str().unwrap_or_default().to_string();
    if qrcode.is_empty() || qr_url.is_empty() {
        return Err("服务端未返回二维码".to_string());
    }
    Ok(ClawbotQrStart {
        qr_svg: qr_svg_data_url(&qr_url),
        qrcode,
        qr_url,
        base_url: CLAWBOT_BASE.to_string(),
    })
}

/// 扫码绑定第 2 步：单次长轮询扫码状态。前端循环调用直到 confirmed / 失败。
#[derive(serde::Serialize)]
pub struct ClawbotQrPoll {
    pub status: String,
    /// IDC 重定向后的新轮询 base，前端下次带回。
    pub base_url: String,
    pub bot_token: Option<String>,
    pub to_user_id: Option<String>,
    pub bot_id: Option<String>,
    /// confirmed 时回传可直接落库的 target（baseurl?to_user_id=..）。
    pub target: Option<String>,
}

pub async fn clawbot_qr_poll(
    qrcode: &str,
    base_url: &str,
    verify_code: Option<&str>,
) -> Result<ClawbotQrPoll, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(40))
        .build()
        .map_err(|e| e.to_string())?;
    let mut url = format!(
        "{}/ilink/bot/get_qrcode_status?qrcode={}",
        base_url.trim_end_matches('/'),
        percent_encode(qrcode)
    );
    if let Some(code) = verify_code {
        if !code.trim().is_empty() {
            url.push_str(&format!("&verify_code={}", percent_encode(code.trim())));
        }
    }
    // 网关/客户端超时一律按 wait 处理，让前端继续轮询。
    let resp = match clawbot_headers(client.get(&url), None).send().await {
        Ok(r) => r,
        Err(_) => return Ok(wait_poll(base_url)),
    };
    if !resp.status().is_success() {
        return Ok(wait_poll(base_url));
    }
    let v: Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => return Ok(wait_poll(base_url)),
    };
    let status = v["status"].as_str().unwrap_or("wait").to_string();
    let mut out = wait_poll(base_url);
    out.status = status.clone();
    match status.as_str() {
        "scaned_but_redirect" => {
            if let Some(host) = v["redirect_host"].as_str() {
                if !host.is_empty() {
                    out.base_url = format!("https://{host}");
                }
            }
        }
        "confirmed" => {
            let bot_id = v["ilink_bot_id"].as_str().unwrap_or_default().to_string();
            if bot_id.is_empty() {
                return Err("登录已确认但服务端缺少 ilink_bot_id".to_string());
            }
            let token = v["bot_token"].as_str().unwrap_or_default().to_string();
            let to_user_id = v["ilink_user_id"].as_str().unwrap_or_default().to_string();
            // baseurl 优先用服务端返回，否则沿用当前轮询 base。
            let send_base = v["baseurl"].as_str().filter(|s| !s.is_empty()).unwrap_or(&out.base_url).to_string();
            out.target = Some(format!("{}?to_user_id={}", send_base, percent_encode(&to_user_id)));
            out.bot_token = Some(token);
            out.to_user_id = Some(to_user_id);
            out.bot_id = Some(bot_id);
        }
        _ => {}
    }
    Ok(out)
}

fn wait_poll(base_url: &str) -> ClawbotQrPoll {
    ClawbotQrPoll {
        status: "wait".to_string(),
        base_url: base_url.to_string(),
        bot_token: None,
        to_user_id: None,
        bot_id: None,
        target: None,
    }
}

/// 把待扫描的 URL 编码成二维码 SVG，再裹成 data URL 供前端 <img> 直接显示。
fn qr_svg_data_url(content: &str) -> String {
    use base64::Engine;
    use qrcode::render::svg;
    use qrcode::{EcLevel, QrCode};
    let svg = QrCode::with_error_correction_level(content.as_bytes(), EcLevel::M)
        .map(|code| {
            code.render::<svg::Color>()
                .min_dimensions(220, 220)
                .quiet_zone(true)
                .dark_color(svg::Color("#16110d"))
                .light_color(svg::Color("#ffffff"))
                .build()
        })
        .unwrap_or_default();
    if svg.is_empty() {
        return String::new();
    }
    format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(svg.as_bytes())
    )
}

/// 仅对 base64/签名里需要转义的字符做百分号编码（足够拼进 query string）。
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
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
            "CREATE TABLE notify_channels (id TEXT PRIMARY KEY, name TEXT, kind TEXT, target TEXT, events TEXT, enabled INTEGER, secret TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO notify_channels (id,name,kind,target,events,enabled,secret) VALUES ('1','t',?,?,?,1,'')",
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

    #[tokio::test]
    async fn dispatch_feishu_shape_over_http() {
        let (url, received) = capture_server().await;
        let pool = mem_pool("feishu", &url, "").await;
        dispatch(&pool, "cr_merged", "标题", "正文").await;
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        let got = received.lock().await;
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["msg_type"], "text");
        assert!(got[0]["content"]["text"].as_str().unwrap().contains("标题"));
    }

    #[test]
    fn payload_shapes_by_kind() {
        assert!(build_payload("slack", "cr_merged", "t", "b").get("text").is_some());
        let wecom = build_payload("wecom", "cr_merged", "t", "b");
        assert_eq!(wecom["msgtype"], "text");
        assert!(wecom["text"]["content"].is_string());
        let feishu = build_payload("feishu", "cr_merged", "t", "b");
        assert_eq!(feishu["msg_type"], "text");
        assert!(feishu["content"]["text"].is_string());
        let ding = build_payload("dingtalk", "cr_merged", "t", "b");
        assert_eq!(ding["msgtype"], "text");
    }

    #[test]
    fn feishu_sign_is_deterministic() {
        // 与飞书官方算法对齐：key = "{ts}\n{secret}", data = 空。
        // 固定输入下结果稳定，可作为回归基线。
        let s = feishu_sign("1599360473", "xxxxxxxxxxxxxxxxxxxxx");
        assert_eq!(s, feishu_sign("1599360473", "xxxxxxxxxxxxxxxxxxxxx"));
        assert!(!s.is_empty());
        // 不同 secret/时间戳应产生不同签名。
        assert_ne!(s, feishu_sign("1599360474", "xxxxxxxxxxxxxxxxxxxxx"));
    }

    #[test]
    fn wechat_uin_is_base64_of_decimal_string() {
        use base64::Engine;
        let uin = wechat_uin();
        let bytes = base64::engine::general_purpose::STANDARD.decode(&uin).unwrap();
        let s = String::from_utf8(bytes).unwrap();
        // 解码后应是纯十进制数字串（对齐 openclaw randomWechatUin）。
        assert!(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit()), "got {s}");
    }

    #[test]
    fn clawbot_send_body_shape() {
        // 复刻 send_to_channel 里 clawbot 分支的 body 形状，断言关键字段。
        let msg = json!({
            "from_user_id": "", "to_user_id": "U123", "client_id": "af-1",
            "message_type": 2, "message_state": 2,
            "item_list": [ { "type": 1, "text_item": { "text": "hi" } } ]
        });
        let payload = json!({ "msg": msg, "base_info": clawbot_base_info() });
        assert_eq!(payload["msg"]["message_type"], 2);
        assert_eq!(payload["msg"]["item_list"][0]["type"], 1);
        assert_eq!(payload["msg"]["item_list"][0]["text_item"]["text"], "hi");
        assert_eq!(payload["base_info"]["bot_agent"], "OpenClaw");
        assert_eq!(payload["base_info"]["channel_version"], "2.4.3");
    }

    #[test]
    fn join_url_no_double_slash() {
        assert_eq!(join_url("https://h", "ilink/bot/sendmessage"), "https://h/ilink/bot/sendmessage");
        assert_eq!(join_url("https://h/", "/ilink/bot/sendmessage"), "https://h/ilink/bot/sendmessage");
    }

    #[test]
    fn qr_svg_is_data_url() {
        let d = qr_svg_data_url("https://example.com/scan?x=1");
        assert!(d.starts_with("data:image/svg+xml;base64,"), "got prefix {}", &d[..30.min(d.len())]);
        assert!(d.len() > 100);
    }

    #[test]
    fn dingtalk_sign_and_encode() {
        let raw = dingtalk_sign("1599360473", "SECxxxxxx");
        assert_eq!(raw, dingtalk_sign("1599360473", "SECxxxxxx"));
        let enc = percent_encode(&raw);
        // base64 里的 +、/、= 必须被转义
        assert!(!enc.contains('+') && !enc.contains('/') && !enc.contains('='));
    }
}
