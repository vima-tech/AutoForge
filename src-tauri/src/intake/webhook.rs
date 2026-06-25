use super::ratelimit::{self, RateLimiter};
use super::{IntakeMode, IntakePayload};
use crate::db::Db;
use crate::models::widget::WidgetToken;
use crate::tasks::runner::JobSender;
use axum::{
    extract::{ConnectInfo, Query, Request, State},
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
    /// 进程内限流器（per-IP / per-project / global 滑窗）。
    limiter: Arc<RateLimiter>,
}

/// 入站 query 参数。`src=widget` 标记匿名反馈 widget 来源：无论命中的 token 是 flow 还是
/// triage，一律强制落入待整理池。该标记**只能把模式降级为 triage、不能提权为 flow**，
/// 因此即便项目 flow token 被公开嵌入 widget HTML，匿名访客也无法借此绕过待整理直进分析队列。
#[derive(Deserialize, Default)]
struct IssueQuery {
    #[serde(default)]
    src: Option<String>,
}

#[derive(Deserialize)]
struct WebhookPayload {
    /// 可选：仅用于防误投校验。真正的项目归属由 token 单向推导，
    /// 若提供且与 token 绑定项目不一致则拒绝（捕获配置错误，杜绝跨项目伪造）。
    #[serde(default)]
    project_id: Option<String>,
    title: String,
    description: Option<String>,
    category: Option<String>,
    severity: Option<String>,
    source_ref: Option<String>,
}

/// 启动 webhook HTTP 服务（绑定 127.0.0.1:{port}，仅本机可访问）。
/// 不再需要全局主 token：每个入站请求按项目级 token（widget_tokens 表）鉴权，
/// 项目与落库模式均由命中的 token 推导。
pub async fn start(port: u16, db: Db, job_tx: JobSender, app: AppHandle) -> anyhow::Result<()> {
    let state = WebhookState {
        db,
        job_tx,
        app,
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

async fn handle_issue(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(ws): State<WebhookState>,
    Query(q): Query<IssueQuery>,
    headers: HeaderMap,
    Json(payload): Json<WebhookPayload>,
) -> (StatusCode, Json<serde_json::Value>) {
    // ── ③ 限流：先于一切重活（含鉴权 DB 查询），与 token 是否泄露无关 ─────────
    // 此刻项目尚未鉴权确定，per-project 键用 payload 提示值（不可信，仅作粗粒度兜底）。
    let ip = client_ip(&headers, peer);
    let proj_hint = payload.project_id.as_deref().unwrap_or("unknown");
    let allowed = ws.limiter.check(&format!("ip:{ip}"), ratelimit::IP_MAX)
        && ws.limiter.check(&format!("proj:{proj_hint}"), ratelimit::PROJECT_MAX)
        && ws.limiter.check("global", ratelimit::GLOBAL_MAX);
    if !allowed {
        warn!("[webhook] rate limited ip={} project={}", ip, proj_hint);
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({"error": "请求过于频繁，请稍后再试"})),
        );
    }

    // ── ② 鉴权：项目级 token；项目与落库模式由命中的 token 推导 ───────────────
    let bearer = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim())
        .unwrap_or("");

    let token = match authenticate(&ws, bearer, payload.project_id.as_deref()).await {
        Some(t) => t,
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

    // ── ① 落库模式决策 ───────────────────────────────────────────────────────────
    //   * src=widget（匿名反馈 widget）：一律强制 triage —— 仅降级、不可提权，
    //     故公开嵌入项目 flow token 也不会让访客绕过待整理直进分析队列。
    //   * 否则按 token.mode：flow → 自动分析；triage → 待整理池（不烧 AI）。
    let is_widget = q.src.as_deref() == Some("widget");
    let (source_type, mode) = if is_widget {
        ("widget", IntakeMode::Triage)
    } else if token.mode == "flow" {
        ("webhook", IntakeMode::Flow)
    } else {
        ("widget", IntakeMode::Triage)
    };

    let intake = IntakePayload {
        // 项目由 token 单向推导，忽略 payload.project_id（已在鉴权阶段校验一致性）。
        project_id: token.project_id.clone(),
        title: payload.title,
        description: payload.description,
        category: payload.category,
        severity: payload.severity,
        source_type: source_type.to_string(),
        source_ref: payload.source_ref,
    };

    let result = super::gateway::receive(&ws.db, &ws.job_tx, &ws.app, intake, mode).await;

    // 投递成功后记录 token 最近使用时间（best-effort，不阻断响应）。
    if result.is_ok() {
        let _ = sqlx::query("UPDATE widget_tokens SET last_used_at = datetime('now') WHERE id=?")
            .bind(&token.id)
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

/// 校验 Bearer token：按 token 精确查 widget_tokens（已发布的 token 无需防时序）。
/// token 必须 enabled、未过期。项目由 token 推导；若 payload 提供了 project_id，
/// 则必须与 token 绑定项目一致（防误投/捕获配置错误），否则拒绝。
async fn authenticate(
    ws: &WebhookState,
    bearer: &str,
    payload_project: Option<&str>,
) -> Option<WidgetToken> {
    if bearer.is_empty() {
        return None;
    }
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
    // 若调用方显式带了 project_id，必须与 token 绑定项目一致（空串视为未提供）。
    if let Some(p) = payload_project {
        if !p.is_empty() && p != wt.project_id {
            warn!(
                "[webhook] token project mismatch: token-bound={} submitted={}",
                wt.project_id, p
            );
            return None;
        }
    }
    Some(wt)
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
