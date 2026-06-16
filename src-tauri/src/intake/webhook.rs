use super::IntakePayload;
use crate::db::Db;
use crate::tasks::runner::JobSender;
use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::net::SocketAddr;
use tauri::AppHandle;
use tracing::{info, warn};

#[derive(Clone)]
struct WebhookState {
    db: Db,
    job_tx: JobSender,
    app: AppHandle,
    token: String,
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
    let state = WebhookState { db, job_tx, app, token };
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
    axum::serve(listener, router)
        .await
        .map_err(|e| anyhow::anyhow!("webhook server error: {}", e))?;
    Ok(())
}

async fn handle_issue(
    State(ws): State<WebhookState>,
    headers: HeaderMap,
    Json(payload): Json<WebhookPayload>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Bearer token 认证（常数时间比较防时序攻击）
    let auth = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    // Reject when no token is configured so an empty secret can never authorize
    // (an empty `ws.token` would otherwise accept the literal "Bearer ").
    if ws.token.is_empty() {
        warn!("[webhook] rejected: no token configured");
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Unauthorized"})),
        );
    }
    let expected = format!("Bearer {}", ws.token);
    if !constant_time_eq(auth.as_bytes(), expected.as_bytes()) {
        warn!("[webhook] unauthorized request");
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Unauthorized"})),
        );
    }

    if payload.title.trim().is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": "title 不能为空"})),
        );
    }

    let intake = IntakePayload {
        project_id: payload.project_id,
        title: payload.title,
        description: payload.description,
        category: payload.category,
        severity: payload.severity,
        source_type: "webhook".to_string(),
        source_ref: payload.source_ref,
    };

    match super::gateway::receive(&ws.db, &ws.job_tx, &ws.app, intake).await {
        Ok(issue) => {
            let val = serde_json::to_value(&issue).unwrap_or_default();
            (StatusCode::CREATED, Json(val))
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        ),
    }
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
