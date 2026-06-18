use super::ratelimit::{self, RateLimiter};
use super::{IntakeMode, IntakePayload};
use crate::db::Db;
use crate::models::widget::WidgetToken;
use crate::tasks::runner::JobSender;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use tauri::AppHandle;
use tracing::{info, warn};

#[derive(Clone)]
struct WebhookState {
    db: Db,
    job_tx: JobSender,
    app: AppHandle,
    /// 主 token：GitHub 同步 / 通用 webhook 共用，命中即按 Flow 自动分析（保持原行为）。
    token: String,
    /// 进程内限流器（per-IP / per-project / global 滑窗）。
    limiter: Arc<RateLimiter>,
}

#[derive(Deserialize)]
struct WebhookPayload {
    project_id: String,
    title: String,
    description: Option<String>,
    category: Option<String>,
    severity: Option<String>,
    source_ref: Option<String>,
}

/// 启动 webhook HTTP 服务（绑定 127.0.0.1:{port}，仅本机可访问）
pub async fn start(
    port: u16,
    token: String,
    db: Db,
    job_tx: JobSender,
    app: AppHandle,
) -> anyhow::Result<()> {
    let state = WebhookState {
        db,
        job_tx,
        app,
        token,
        limiter: Arc::new(RateLimiter::new(ratelimit::WINDOW)),
    };
    let router = Router::new()
        .route("/webhook/issues", post(handle_issue).options(preflight))
        .route("/widget.js", get(serve_widget))
        .layer(middleware::from_fn(add_cors))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("无法绑定 webhook 端口 {}: {}", port, e))?;

    info!("[webhook] server listening on {}", addr);
    // into_make_service_with_connect_info：让 handler 能拿到对端地址用于 per-IP 限流。
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .map_err(|e| anyhow::anyhow!("webhook server error: {}", e))?;
    Ok(())
}

/// 解析客户端 IP：优先取 `X-Forwarded-For` 首跳（反向代理/隧道暴露公网时对端是代理，
/// 真实来源在 XFF 里），否则回退到 TCP 对端地址。
fn client_ip(headers: &HeaderMap, peer: SocketAddr) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| peer.ip().to_string())
}

/// widget token 鉴权后的来源判定。
enum AuthSource {
    /// 主 token：GitHub/通用 webhook，自动进分析队列。
    Master,
    /// widget token：匿名反馈，强制进 Triage 待整理池，且限定其绑定项目。
    Widget(WidgetToken),
}

async fn handle_issue(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(ws): State<WebhookState>,
    headers: HeaderMap,
    Json(payload): Json<WebhookPayload>,
) -> (StatusCode, Json<serde_json::Value>) {
    // ── ③ 限流：先于一切重活，与 token 是否泄露无关 ──────────────────────────
    let ip = client_ip(&headers, peer);
    let allowed = ws.limiter.check(&format!("ip:{ip}"), ratelimit::IP_MAX)
        && ws
            .limiter
            .check(&format!("proj:{}", payload.project_id), ratelimit::PROJECT_MAX)
        && ws.limiter.check("global", ratelimit::GLOBAL_MAX);
    if !allowed {
        warn!("[webhook] rate limited ip={} project={}", ip, payload.project_id);
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({"error": "请求过于频繁，请稍后再试"})),
        );
    }

    // ── ② 鉴权：主 token(Flow) 或 widget token(Triage + 项目归属) ───────────
    let bearer = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim())
        .unwrap_or("");

    let source = match authenticate(&ws, bearer, &payload.project_id).await {
        Some(s) => s,
        None => {
            warn!("[webhook] unauthorized request ip={}", ip);
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Unauthorized"})),
            );
        }
    };

    if payload.title.trim().is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": "title 不能为空"})),
        );
    }

    // ── ① 来源决定落库模式：widget → Triage 待整理池（不自动烧 AI 预算）──────
    let (source_type, mode) = match &source {
        AuthSource::Master => ("webhook", IntakeMode::Flow),
        AuthSource::Widget(_) => ("widget", IntakeMode::Triage),
    };

    let intake = IntakePayload {
        project_id: payload.project_id,
        title: payload.title,
        description: payload.description,
        category: payload.category,
        severity: payload.severity,
        source_type: source_type.to_string(),
        source_ref: payload.source_ref,
    };

    let result = super::gateway::receive(&ws.db, &ws.job_tx, &ws.app, intake, mode).await;

    // widget token 投递成功后记录最近使用时间（best-effort，不阻断响应）。
    if let (AuthSource::Widget(wt), Ok(_)) = (&source, &result) {
        let _ = sqlx::query("UPDATE widget_tokens SET last_used_at = datetime('now') WHERE id=?")
            .bind(&wt.id)
            .execute(&ws.db)
            .await;
    }

    match result {
        Ok(issue) => {
            let val = serde_json::to_value(&issue).unwrap_or_default();
            (StatusCode::CREATED, Json(val))
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))),
    }
}

/// 校验 Bearer token：先比主 token（常数时间），未命中再查 widget_tokens。
/// widget token 必须 enabled、未过期，且绑定项目与提交项目一致。
async fn authenticate(ws: &WebhookState, bearer: &str, project_id: &str) -> Option<AuthSource> {
    if bearer.is_empty() {
        return None;
    }
    // 主 token：仅在已配置时才尝试，空主 token 不得授权。
    if !ws.token.is_empty() && constant_time_eq(bearer.as_bytes(), ws.token.as_bytes()) {
        return Some(AuthSource::Master);
    }
    // widget token：直接按 token 精确查（已发布的 token 无需防时序）。
    let wt = sqlx::query_as::<_, WidgetToken>(
        "SELECT * FROM widget_tokens
         WHERE token = ? AND enabled = 1
           AND (expires_at IS NULL OR expires_at > datetime('now'))",
    )
    .bind(bearer)
    .fetch_optional(&ws.db)
    .await
    .ok()
    .flatten()?;
    // 项目归属校验：一个 widget token 只能为它绑定的项目投递反馈。
    if wt.project_id != project_id {
        warn!(
            "[webhook] widget token project mismatch: token-bound={} submitted={}",
            wt.project_id, project_id
        );
        return None;
    }
    Some(AuthSource::Widget(wt))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ── M10 embeddable widget ──────────────────────────────────────────────────────
const WIDGET_JS: &str = include_str!("../../assets/widget.js");

async fn serve_widget() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        WIDGET_JS,
    )
}

async fn preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}

/// Permissive CORS so the widget can be embedded on any origin and POST feedback.
async fn add_cors(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        header::HeaderValue::from_static("*"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        header::HeaderValue::from_static("authorization,content-type"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        header::HeaderValue::from_static("POST,GET,OPTIONS"),
    );
    resp
}
