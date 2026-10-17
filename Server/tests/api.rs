//! wtmd-server 集成测试：进程内构建 router（tower::ServiceExt::oneshot）。
//!
//! 覆盖：report→devices 字段/online、鉴权 40100、空 api_token、非法体 40000、
//! history+limit、public config 形状、限流 42900+Retry-After、离线判定（received_at）、
//! SSE 双帧广播、静态托管回退、probe 形状。
//!
//! 全部使用临时目录，绝不触碰 Server\config.json。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use serde_json::{Value, json};
use tokio_stream::StreamExt;
use tower::ServiceExt;

use wtmd_server::state::AppState;
use wtmd_server::{Config, build_router, build_router_with_state};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 独立临时目录（每次调用唯一）。
///
/// 使用 `Server\tests\.tmp` 下的目录而非系统 %TEMP%：测试进程在受限沙箱下对
/// 系统临时目录无写权限，且目录位于本 crate 写入范围内；测试结束后可安全删除。
fn temp_dir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join(".tmp");
    let dir = base.join(format!("{}-{n}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn test_config(dir: &Path) -> Config {
    Config {
        db_path: dir.join("test.db").to_string_lossy().into_owned(),
        api_token: "test-token".to_string(),
        web_dir: dir.join("web").to_string_lossy().into_owned(),
        ..Config::default()
    }
}

fn api_request(
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<String>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    builder.body(Body::from(body.unwrap_or_default())).unwrap()
}

/// 发请求并解析 envelope JSON。
async fn call(
    app: &axum::Router,
    req: Request<Body>,
) -> (StatusCode, HeaderMap, Value) {
    let response = app.clone().oneshot(req).await.expect("oneshot");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, value)
}

fn report_body(device_id: &str, status: &str, timestamp: &str) -> String {
    json!({
        "device_id": device_id,
        "device_name": "书房电脑",
        "status": status,
        "timestamp": timestamp,
        "app": {
            "process_name": "code.exe",
            "display_name": "VS Code",
            "window_title": "main.rs"
        }
    })
    .to_string()
}

const TS: &str = "2026-01-01T00:00:00Z";

// ----- (1) report → devices -------------------------------------------------

#[tokio::test]
async fn report_updates_public_and_v1_devices() {
    let dir = temp_dir("devices");
    let app = build_router(test_config(&dir)).unwrap();

    let (status, _headers, body) = call(
        &app,
        api_request(
            Method::POST,
            "/api/v1/report",
            Some("test-token"),
            Some(report_body("pc-1", "active", TS)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["code"], 0);
    assert_eq!(body["message"], "ok");
    assert_eq!(body["data"]["ok"], true);

    // 公开设备列表：契约 12 字段。
    let (status, _headers, body) = call(&app, api_request(Method::GET, "/api/public/devices", None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["code"], 0);
    let devices = body["data"]["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 1);
    let device = &devices[0];
    let keys: BTreeSet<String> = device
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let expected: BTreeSet<String> = [
        "id", "name", "enabled", "online", "status", "app", "timestamp",
        "latency_ms", "http_status", "error", "updated_at", "healthy",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(keys, expected, "public device 字段必须恰好是契约的 12 个");
    assert_eq!(device["id"], "pc-1");
    assert_eq!(device["name"], "书房电脑");
    assert_eq!(device["enabled"], true);
    assert_eq!(device["online"], true);
    assert_eq!(device["status"], "active");
    assert_eq!(device["app"]["process_name"], "code.exe");
    assert_eq!(device["app"]["display_name"], "VS Code");
    assert_eq!(device["app"]["window_title"], "main.rs");
    assert_eq!(device["timestamp"], TS);
    assert!(device["latency_ms"].is_null());
    assert!(device["http_status"].is_null());
    assert!(device["error"].is_null());
    assert!(device["updated_at"].is_number());
    assert_eq!(device["healthy"], true);

    // v1 设备列表（Bearer）。
    let (status, _headers, body) = call(
        &app,
        api_request(Method::GET, "/api/v1/devices", Some("test-token"), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let device = &body["data"]["devices"][0];
    assert_eq!(device["device_id"], "pc-1");
    assert_eq!(device["device_name"], "书房电脑");
    assert_eq!(device["status"], "active");
    assert_eq!(device["online"], true);
    assert_eq!(device["timestamp"], TS);
    assert_eq!(device["app"]["display_name"], "VS Code");
}

// ----- (2) 鉴权 ------------------------------------------------------------

#[tokio::test]
async fn wrong_or_missing_token_is_401_40100() {
    let dir = temp_dir("auth");
    let app = build_router(test_config(&dir)).unwrap();

    for (label, token) in [("缺失", None), ("错误", Some("wrong-token"))] {
        let (status, _headers, body) = call(
            &app,
            api_request(Method::GET, "/api/v1/devices", token, None),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{label} token 应 401");
        assert_eq!(body["code"], 40100, "{label} token 应 code 40100");
        assert!(body["data"].is_null());

        let (status, _headers, body) = call(
            &app,
            api_request(
                Method::POST,
                "/api/v1/report",
                token,
                Some(report_body("pc-1", "active", TS)),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{label} token report 应 401");
        assert_eq!(body["code"], 40100);

        for uri in ["/api/v1/status", "/api/v1/devices/pc-1", "/api/v1/devices/pc-1/history"] {
            let (status, _headers, body) =
                call(&app, api_request(Method::GET, uri, token, None)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{label} token {uri} 应 401");
            assert_eq!(body["code"], 40100);
        }
    }

    // 正确 token 通过。
    let (status, _headers, body) = call(
        &app,
        api_request(Method::GET, "/api/v1/devices", Some("test-token"), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["code"], 0);

    // 公开接口无鉴权。
    let (status, _headers, body) = call(&app, api_request(Method::GET, "/api/public/config", None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["code"], 0);
}

// ----- (3) 空 api_token ----------------------------------------------------

#[tokio::test]
async fn empty_server_token_rejects_protected_endpoints() {
    let dir = temp_dir("empty-token");
    let mut config = test_config(&dir);
    config.api_token = String::new();
    let app = build_router(config).unwrap();

    for token in [None, Some("anything"), Some("")] {
        let (status, _headers, body) = call(
            &app,
            api_request(Method::GET, "/api/v1/devices", token, None),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["code"], 40100);

        let (status, _headers, body) = call(
            &app,
            api_request(
                Method::POST,
                "/api/v1/report",
                token,
                Some(report_body("pc-1", "active", TS)),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "report 也必须 401");
        assert_eq!(body["code"], 40100);
    }

    // 健康检查仍然公开。
    let (status, _headers, body) = call(&app, api_request(Method::GET, "/api/v1/health", None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["status"], "ok");
}

// ----- (4) 非法体 ----------------------------------------------------------

#[tokio::test]
async fn invalid_bodies_are_400_40000() {
    let dir = temp_dir("bad-body");
    let app = build_router(test_config(&dir)).unwrap();

    // 非法 JSON。
    let (status, _headers, body) = call(
        &app,
        api_request(Method::POST, "/api/v1/report", Some("test-token"), Some("{nope".into())),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], 40000);

    // device_id 为空（沿用原基线：先于鉴权检查）。
    let (status, _headers, body) = call(
        &app,
        api_request(
            Method::POST,
            "/api/v1/report",
            None,
            Some(json!({"device_id": "  ", "status": "active", "timestamp": TS}).to_string()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], 40000);

    // 非法 status。
    let (status, _headers, body) = call(
        &app,
        api_request(
            Method::POST,
            "/api/v1/report",
            Some("test-token"),
            Some(json!({"device_id": "pc-1", "status": "flying", "timestamp": TS}).to_string()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], 40000);

    // 缺少 timestamp。
    let (status, _headers, body) = call(
        &app,
        api_request(
            Method::POST,
            "/api/v1/report",
            Some("test-token"),
            Some(json!({"device_id": "pc-1", "status": "active"}).to_string()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], 40000);

    // timestamp 不可解析。
    let (status, _headers, body) = call(
        &app,
        api_request(
            Method::POST,
            "/api/v1/report",
            Some("test-token"),
            Some(json!({"device_id": "pc-1", "status": "active", "timestamp": "昨天"}).to_string()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], 40000);

    // 未上报任何设备：未受影响。
    let (status, _headers, body) = call(&app, api_request(Method::GET, "/api/public/devices", None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["data"]["devices"].as_array().unwrap().is_empty());
}

// ----- (5) history 与 limit -------------------------------------------------

#[tokio::test]
async fn history_returns_newest_first_and_respects_limit() {
    let dir = temp_dir("history");
    let app = build_router(test_config(&dir)).unwrap();

    for (i, ts) in [
        (0, "2026-01-01T00:00:00Z"),
        (1, "2026-01-01T00:01:00Z"),
        (2, "2026-01-01T00:02:00Z"),
    ] {
        let body = json!({
            "device_id": "pc-2",
            "device_name": "书房电脑",
            "status": if i == 2 { "paused" } else { "active" },
            "timestamp": ts,
            "app": {
                "process_name": format!("app{i}.exe"),
                "display_name": format!("应用 {i}"),
                "window_title": format!("窗口 {i}")
            }
        })
        .to_string();
        let (status, _headers, resp) = call(
            &app,
            api_request(Method::POST, "/api/v1/report", Some("test-token"), Some(body)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp["code"], 0);
    }

    // v1 history：默认 limit=50，按 id DESC（最新在前）。
    let (status, _headers, body) = call(
        &app,
        api_request(Method::GET, "/api/v1/devices/pc-2/history", Some("test-token"), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entries = body["data"]["history"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(body["data"]["device_id"], "pc-2");
    assert_eq!(entries[0]["timestamp"], "2026-01-01T00:02:00Z");
    assert_eq!(entries[0]["status"], "paused");
    assert_eq!(entries[1]["timestamp"], "2026-01-01T00:01:00Z");
    assert_eq!(entries[2]["timestamp"], "2026-01-01T00:00:00Z");
    // v1 条目字段。
    let entry = &entries[0];
    for key in [
        "id", "device_id", "device_name", "status", "process_name",
        "display_name", "window_title", "timestamp",
    ] {
        assert!(entry.get(key).is_some(), "v1 history 条目缺少 {key}");
    }
    assert_eq!(entries[0]["display_name"], "应用 2");

    // limit=2。
    let (status, _headers, body) = call(
        &app,
        api_request(Method::GET, "/api/v1/devices/pc-2/history?limit=2", Some("test-token"), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entries = body["data"]["history"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["timestamp"], "2026-01-01T00:02:00Z");

    // public history：恰好 4 键。
    let (status, _headers, body) = call(
        &app,
        api_request(Method::GET, "/api/public/devices/pc-2/history?limit=1", None, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entries = body["data"]["history"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    let keys: BTreeSet<String> = entries[0].as_object().unwrap().keys().cloned().collect();
    let expected: BTreeSet<String> = ["display_name", "process_name", "status", "timestamp"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(keys, expected, "public history 条目必须恰好 4 键");
    assert_eq!(entries[0]["timestamp"], "2026-01-01T00:02:00Z");

    // 非法 limit。
    for uri in [
        "/api/v1/devices/pc-2/history?limit=0",
        "/api/v1/devices/pc-2/history?limit=1001",
        "/api/v1/devices/pc-2/history?limit=abc",
    ] {
        let (status, _headers, body) =
            call(&app, api_request(Method::GET, uri, Some("test-token"), None)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri} 应 400");
        assert_eq!(body["code"], 40000);
    }

    // 未知设备 → 40400。
    let (status, _headers, body) = call(
        &app,
        api_request(Method::GET, "/api/v1/devices/ghost/history", Some("test-token"), None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], 40400);
    let (status, _headers, body) = call(
        &app,
        api_request(Method::GET, "/api/public/devices/ghost/history", None, None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], 40400);

    // v1 单设备 40400。
    let (status, _headers, body) = call(
        &app,
        api_request(Method::GET, "/api/v1/devices/ghost", Some("test-token"), None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], 40400);
}

// ----- (6) public config ----------------------------------------------------

#[tokio::test]
async fn public_config_matches_web_contract() {
    let dir = temp_dir("config-shape");
    let mut config = test_config(&dir);
    config.page_title = "标题 X".to_string();
    config.page_subtitle = "副标题 Y".to_string();
    config.refresh_interval_seconds = 0; // <1 按 1
    config.show_history = false;
    let app = build_router(config).unwrap();

    let (status, _headers, body) = call(&app, api_request(Method::GET, "/api/public/config", None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["code"], 0);
    let data = &body["data"];
    let keys: BTreeSet<String> = data.as_object().unwrap().keys().cloned().collect();
    let expected: BTreeSet<String> =
        ["page_title", "page_subtitle", "refresh_interval_seconds", "show_history"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    assert_eq!(keys, expected, "public config 必须恰好 4 字段");
    assert_eq!(data["page_title"], "标题 X");
    assert_eq!(data["page_subtitle"], "副标题 Y");
    assert_eq!(data["refresh_interval_seconds"], 1);
    assert_eq!(data["show_history"], false);

    // probe 形状。
    let (status, _headers, body) = call(
        &app,
        Request::builder()
            .method(Method::GET)
            .uri("/api/public/probe")
            .header(header::HOST, "example.test:8080")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let keys: BTreeSet<String> = body["data"].as_object().unwrap().keys().cloned().collect();
    let expected: BTreeSet<String> = ["method", "path", "scheme", "host"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(keys, expected);
    assert_eq!(body["data"]["method"], "GET");
    assert_eq!(body["data"]["path"], "/api/public/probe");
    assert_eq!(body["data"]["scheme"], "http");
    assert_eq!(body["data"]["host"], "example.test:8080");
}

// ----- (7) 限流 -------------------------------------------------------------

#[tokio::test]
async fn rate_limit_returns_429_42900_with_retry_after() {
    let dir = temp_dir("ratelimit");
    let mut config = test_config(&dir);
    config.rate_limit_per_minute = 2;
    let app = build_router(config).unwrap();

    // 前 2 个请求放行，带 X-RateLimit 头。
    for expected_remaining in [1, 0] {
        let (status, headers, body) =
            call(&app, api_request(Method::GET, "/api/public/probe", None, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["code"], 0);
        assert_eq!(
            headers.get("x-ratelimit-limit").and_then(|v| v.to_str().ok()),
            Some("2")
        );
        assert_eq!(
            headers.get("x-ratelimit-remaining").and_then(|v| v.to_str().ok()),
            Some(expected_remaining.to_string().as_str())
        );
    }

    // 第 3 个 → 429。
    let (status, headers, body) = call(&app, api_request(Method::GET, "/api/public/probe", None, None)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["code"], 42900);
    assert!(body["data"].is_null());
    let retry_after = headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .expect("Retry-After 头必须存在");
    let retry_after: u64 = retry_after.parse().expect("Retry-After 为秒数");
    assert!(retry_after >= 1);
    assert_eq!(
        headers.get("x-ratelimit-limit").and_then(|v| v.to_str().ok()),
        Some("2")
    );

    // 限流对 /api/v1 同样生效。
    let (status, _headers, body) = call(
        &app,
        api_request(Method::GET, "/api/v1/health", None, None),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["code"], 42900);
}

// ----- (8) 离线判定 ---------------------------------------------------------

#[tokio::test]
async fn offline_judged_on_received_at() {
    let dir = temp_dir("offline");
    let state = AppState::new(test_config(&dir)).unwrap();
    let app = build_router_with_state(state.clone()).unwrap();

    // 新上报 → online=true。
    let (status, _headers, _body) = call(
        &app,
        api_request(
            Method::POST,
            "/api/v1/report",
            Some("test-token"),
            Some(report_body("pc-3", "active", TS)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _headers, body) = call(&app, api_request(Method::GET, "/api/public/devices", None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["devices"][0]["online"], true);

    // 把接收时间拨回 2 小时 → online=false（离线判定用 received_at）。
    {
        let mut devices = state.devices.write().unwrap();
        let record = devices.get_mut("pc-3").unwrap();
        record.received_at = chrono::Utc::now() - chrono::Duration::seconds(7200);
    }
    let (status, _headers, body) = call(&app, api_request(Method::GET, "/api/public/devices", None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["devices"][0]["online"], false);

    // v1 同样判离线。
    let (status, _headers, body) = call(
        &app,
        api_request(Method::GET, "/api/v1/devices/pc-3", Some("test-token"), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["online"], false);

    // 旧的客户端 timestamp 但刚接收 → 仍 online（判据是 received_at）。
    let (status, _headers, _body) = call(
        &app,
        api_request(
            Method::POST,
            "/api/v1/report",
            Some("test-token"),
            Some(report_body("pc-4", "active", "2020-01-01T00:00:00Z")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _headers, body) = call(&app, api_request(Method::GET, "/api/public/devices", None, None)).await;
    assert_eq!(status, StatusCode::OK);
    let devices = body["data"]["devices"].as_array().unwrap();
    let pc4 = devices.iter().find(|d| d["id"] == "pc-4").unwrap();
    assert_eq!(pc4["online"], true);
    assert_eq!(pc4["timestamp"], "2020-01-01T00:00:00Z");
}

// ----- SSE 双帧 ------------------------------------------------------------

#[tokio::test]
async fn stream_sends_dual_frames_and_broadcasts_reports() {
    let dir = temp_dir("sse");
    let app = build_router(test_config(&dir)).unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/public/stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(content_type.starts_with("text/event-stream"), "got {content_type}");

    let mut stream = response.into_body().into_data_stream();
    let mut buffer = String::new();
    // 初始快照应立即到达（双帧：event: update 帧 + 默认 message 帧）。
    // 注：serde_json 默认 Map 为 BTreeMap，键按字典序输出，故按子串断言而非整串。
    while !(buffer.contains("event: update")
        && buffer.contains("\"type\":\"devices\"")
        && buffer.matches("data: {").count() >= 2)
    {
        let chunk = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .unwrap_or_else(|_| panic!("SSE 初始帧超时, buffer={buffer:?}"))
            .expect("SSE 流不应结束")
            .expect("SSE 帧读取失败");
        buffer.push_str(&String::from_utf8_lossy(&chunk));
    }

    // 上报后广播应到达。
    let (status, _headers, _body) = call(
        &app,
        api_request(
            Method::POST,
            "/api/v1/report",
            Some("test-token"),
            Some(report_body("pc-9", "active", TS)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    while !buffer.contains("\"id\":\"pc-9\"") {
        let chunk = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .unwrap_or_else(|_| panic!("SSE 广播帧超时, buffer={buffer:?}"))
            .expect("SSE 流不应结束")
            .expect("SSE 帧读取失败");
        buffer.push_str(&String::from_utf8_lossy(&chunk));
    }
}

// ----- 静态托管 -------------------------------------------------------------

#[tokio::test]
async fn static_files_and_spa_fallback() {
    let dir = temp_dir("static");
    let web = dir.join("web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "<html>WTMD</html>").unwrap();
    std::fs::write(web.join("app.js"), "// app").unwrap();
    let app = build_router(test_config(&dir)).unwrap();

    for (uri, expected) in [
        ("/", "WTMD"),
        ("/index.html", "WTMD"),
        ("/app.js", "// app"),
        ("/some/spa/route", "WTMD"), // 非 /api 未知路径回退 index.html
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri} 应回 200");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains(expected), "{uri} 应含 {expected}，got {text}");
    }

    // 未知 /api 路径回 404 envelope，不回 index.html。
    let (status, _headers, body) = call(&app, api_request(Method::GET, "/api/v1/nope", None, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], 40400);
    let (status, _headers, body) = call(&app, api_request(Method::GET, "/api/nope/deep", None, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], 40400);
}
