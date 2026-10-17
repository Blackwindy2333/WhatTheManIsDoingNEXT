//! 路由：envelope v1 API + 公开只读 API + 静态托管。
//!
//! 限流（按客户端 IP）应用于全部 `/api/*`；`/api` 未知路径回 404 envelope；
//! 非 `/api` 未知路径回退 `index.html`（SPA）。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{ConnectInfo, Json, Path as PathParam, Query, Request, State};
use axum::http::{HeaderMap, Method, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::Router;
use serde_json::{Value, json};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};

use crate::config::Config;
use crate::error::{ApiError, ok};
use crate::sse::SseFrame;
use crate::state::{AppState, ReportInput, now_iso};

/// 组装完整路由（API + 静态托管 + CORS）。
pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/api/v1/report", post(report))
        .route("/api/v1/health", get(health))
        .route("/api/v1/status", get(v1_status))
        .route("/api/v1/devices", get(v1_devices))
        .route("/api/v1/devices/{id}", get(v1_device))
        .route("/api/v1/devices/{id}/history", get(v1_history))
        .route("/api/public/config", get(public_config))
        .route("/api/public/probe", get(public_probe))
        .route("/api/public/devices", get(public_devices))
        .route("/api/public/devices/{id}/history", get(public_history))
        .route("/api/public/stream", get(public_stream))
        .route("/api", any(api_not_found))
        .route("/api/{*path}", any(api_not_found))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ));

    let web_dir = resolve_web_dir(&state.config.web_dir);
    let index = web_dir.join("index.html");
    let static_service = ServeDir::new(&web_dir).fallback(ServeFile::new(index));

    Router::new()
        .merge(api)
        .fallback_service(static_service)
        .layer(cors_layer(&state.config))
        .with_state(state)
}

/// 静态目录解析：配置值存在则用之，否则试 `../Web`，再回落配置值。
pub fn resolve_web_dir(configured: &str) -> PathBuf {
    let configured = Path::new(configured);
    if configured.is_dir() {
        return configured.to_path_buf();
    }
    let alt = Path::new("..").join("Web");
    if alt.is_dir() {
        return alt;
    }
    configured.to_path_buf()
}

fn cors_layer(config: &Config) -> CorsLayer {
    let origins: Vec<&str> = config
        .cors_origins
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let mut cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .allow_credentials(false);
    if origins.is_empty() || origins.iter().any(|o| *o == "*") {
        cors = cors.allow_origin(Any);
    } else {
        let values: Vec<_> = origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        cors = cors.allow_origin(values);
    }
    cors
}

// ----- 限流 ---------------------------------------------------------------

/// 客户端 IP：trust_proxy=true 时优先 X-Forwarded-For 首跳，否则 X-Real-IP，否则 socket 地址。
fn client_ip(req: &Request, trust_proxy: bool) -> String {
    if trust_proxy {
        if let Some(v) = req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(first) = v.split(',').next() {
                let trimmed = first.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
        if let Some(v) = req.headers().get("x-real-ip").and_then(|v| v.to_str().ok()) {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

async fn rate_limit_middleware(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let key = client_ip(&req, state.config.trust_proxy);
    let decision = state.limiter.check(&key);
    if !decision.allowed {
        return ApiError::rate_limited(
            decision.retry_after,
            decision.limit,
            decision.remaining,
            decision.reset,
        )
        .into_response();
    }
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    if let Ok(value) = decision.limit.to_string().parse() {
        headers.insert("x-ratelimit-limit", value);
    }
    if let Ok(value) = decision.remaining.to_string().parse() {
        headers.insert("x-ratelimit-remaining", value);
    }
    if let Ok(value) = decision.reset.to_string().parse() {
        headers.insert("x-ratelimit-reset", value);
    }
    response
}

// ----- 鉴权 ---------------------------------------------------------------

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = raw.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Bearer <api_token>；token 为空或不匹配一律 401/40100（空 api_token 表示拒绝所有调用）。
fn check_auth(config: &Config, headers: &HeaderMap) -> Result<(), ApiError> {
    let expected = config.api_token.trim();
    if expected.is_empty() {
        return Err(ApiError::unauthorized("api_token is not configured"));
    }
    match extract_bearer(headers) {
        Some(token) if token == expected => Ok(()),
        _ => Err(ApiError::unauthorized("invalid api token")),
    }
}

// ----- /api/v1 ------------------------------------------------------------

/// POST /api/v1/report — 校验顺序沿用原 Python 基线：device_id → 鉴权 → 其余字段。
async fn report(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<Value>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let value = match body {
        Ok(Json(value)) => value,
        Err(rejection) => {
            return ApiError::bad_request(format!("invalid JSON body: {rejection}")).into_response();
        }
    };

    let device_id = value
        .get("device_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if device_id.is_empty() {
        return ApiError::bad_request("device_id is required").into_response();
    }

    if let Err(err) = check_auth(&state.config, &headers) {
        return err.into_response();
    }

    let input = match ReportInput::from_value(&value) {
        Ok(input) => input,
        Err(message) => return ApiError::bad_request(message).into_response(),
    };

    if let Err(err) = state.record_report(&input) {
        return err.into_response();
    }

    let payload = state.devices_message();
    let _ = state.broadcast.send(SseFrame::Update(payload.clone()));
    let _ = state.broadcast.send(SseFrame::Message(payload));

    ok(json!({ "ok": true })).into_response()
}

/// GET /api/v1/health — 无鉴权。
async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ok(json!({
        "status": "ok",
        "time": now_iso(),
        "rate_limit_per_minute": state.config.rate_limit_per_minute,
    }))
}

/// GET /api/v1/status — 服务器信息 + 设备概要。
async fn v1_status(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(err) = check_auth(&state.config, &headers) {
        return err.into_response();
    }
    let devices = state.v1_devices_array();
    let online_count = devices
        .iter()
        .filter(|d| d.get("online").and_then(Value::as_bool).unwrap_or(false))
        .count();
    ok(json!({
        "status": "ok",
        "time": now_iso(),
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": (chrono::Utc::now() - state.started_at).num_seconds(),
        "device_count": devices.len(),
        "online_count": online_count,
        "devices": devices,
    }))
    .into_response()
}

/// GET /api/v1/devices
async fn v1_devices(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(err) = check_auth(&state.config, &headers) {
        return err.into_response();
    }
    ok(json!({ "devices": state.v1_devices_array() })).into_response()
}

/// GET /api/v1/devices/{id}
async fn v1_device(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    PathParam(device_id): PathParam<String>,
) -> Response {
    if let Err(err) = check_auth(&state.config, &headers) {
        return err.into_response();
    }
    let records = state.devices_list();
    let record = records.iter().find(|r| r.device_id == device_id);
    match record {
        Some(record) => {
            let json = crate::state::v1_device_json(
                record,
                chrono::Utc::now(),
                state.config.offline_after_seconds,
            );
            ok(json).into_response()
        }
        None => ApiError::not_found("device not found").into_response(),
    }
}

/// GET /api/v1/devices/{id}/history?limit=
async fn v1_history(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    PathParam(device_id): PathParam<String>,
    query: Result<Query<HashMap<String, String>>, axum::extract::rejection::QueryRejection>,
) -> Response {
    if let Err(err) = check_auth(&state.config, &headers) {
        return err.into_response();
    }
    let limit = match parse_limit(query, 50, 1000) {
        Ok(limit) => limit,
        Err(err) => return err.into_response(),
    };
    history_response(&state, &device_id, limit, HistoryShape::V1)
}

/// GET /api/v1/... 未知路径。
async fn api_not_found() -> ApiError {
    ApiError::not_found("not found")
}

// ----- /api/public ----------------------------------------------------------

/// GET /api/public/config — Web/ 前端契约：4 个字段。
async fn public_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = &state.config;
    ok(json!({
        "page_title": config.page_title,
        "page_subtitle": config.page_subtitle,
        "refresh_interval_seconds": config.refresh_interval_seconds.max(1),
        "show_history": config.show_history,
    }))
}

/// GET /api/public/probe — 连通性探测（契约：method/path/scheme/host）。
async fn public_probe(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let (parts, _body) = req.into_parts();
    let scheme = if state.config.trust_proxy {
        parts
            .headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "http".to_string())
    } else {
        "http".to_string()
    };
    let host = parts
        .headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    ok(json!({
        "method": parts.method.as_str(),
        "path": parts.uri.path(),
        "scheme": scheme,
        "host": host,
    }))
    .into_response()
}

/// GET /api/public/devices — Web/ 前端契约的 12 字段设备列表。
async fn public_devices(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ok(json!({ "devices": state.public_devices_array() }))
}

/// GET /api/public/devices/{id}/history?limit=
async fn public_history(
    State(state): State<Arc<AppState>>,
    PathParam(device_id): PathParam<String>,
    query: Result<Query<HashMap<String, String>>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let limit = match parse_limit(query, 50, 1000) {
        Ok(limit) => limit,
        Err(err) => return err.into_response(),
    };
    history_response(&state, &device_id, limit, HistoryShape::Public)
}

/// GET /api/public/stream — SSE。
async fn public_stream(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    crate::sse::sse_stream(state)
}

// ----- 公共片段 -------------------------------------------------------------

enum HistoryShape {
    V1,
    Public,
}

fn history_response(
    state: &AppState,
    device_id: &str,
    limit: i64,
    shape: HistoryShape,
) -> Response {
    let rows = match state.history(device_id, limit) {
        Ok(rows) => rows,
        Err(err) => {
            return ApiError::internal(format!("history query failed: {err}")).into_response();
        }
    };
    if rows.is_empty() && !state.device_exists(device_id) {
        return ApiError::not_found("device not found").into_response();
    }
    let entries: Vec<Value> = match shape {
        HistoryShape::V1 => rows.iter().map(|row| row.to_v1_json()).collect(),
        HistoryShape::Public => rows.iter().map(|row| row.to_public_json()).collect(),
    };
    ok(json!({ "device_id": device_id, "history": entries })).into_response()
}

fn parse_limit(
    query: Result<Query<HashMap<String, String>>, axum::extract::rejection::QueryRejection>,
    default: i64,
    max: i64,
) -> Result<i64, ApiError> {
    let params = match query {
        Ok(Query(params)) => params,
        Err(rejection) => {
            return Err(ApiError::bad_request(format!("invalid query: {rejection}")));
        }
    };
    match params.get("limit") {
        None => Ok(default),
        Some(raw) => {
            let value: i64 = raw
                .trim()
                .parse()
                .map_err(|_| ApiError::bad_request("limit must be an integer"))?;
            if !(1..=max).contains(&value) {
                return Err(ApiError::bad_request(format!(
                    "limit must be between 1 and {max}"
                )));
            }
            Ok(value)
        }
    }
}
