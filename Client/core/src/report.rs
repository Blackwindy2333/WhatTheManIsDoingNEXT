//! HTTP 上报与结果分类（PLAN.md 阶段 1 / §10：429 按 Retry-After 退避、传输失败重试）。
//!
//! HTTP 语义结果用 `Ok(ReportOutcome)` 表达；传输层失败用 `Err(ReportError::Network)`。
//! [`HttpPort`] 可注入 mock 以便测试。

use crate::config::AppConfig;
use crate::protocol::ReportPayload;
use std::fmt;

/// 上报的 HTTP 语义结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReportOutcome {
    /// 2xx。
    Sent,
    /// 401/403：token 缺失或不匹配。
    Unauthorized,
    /// 400：请求体不合法。
    BadRequest,
    /// 429：触发限流，`Retry-After`（秒）解析失败为 `None`。
    RateLimited { retry_after_secs: Option<u64> },
    /// 其余（5xx 等）：服务端错误。
    ServerError,
}

/// 上报错误：仅传输层失败与请求体编码失败走 `Err`。
#[derive(Debug)]
pub enum ReportError {
    /// 网络/传输失败。
    Network(String),
    /// JSON 编码失败。
    Decode(String),
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReportError::Network(msg) => write!(f, "report network error: {msg}"),
            ReportError::Decode(msg) => write!(f, "report decode error: {msg}"),
        }
    }
}

impl std::error::Error for ReportError {}

/// HTTP 响应快照。
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// 可注入的 HTTP 传输端口（`POST JSON` + Bearer）。
///
/// 实现约定：收到 HTTP 响应（无论状态码）返回 `Ok(HttpResponse)`；
/// 仅传输层失败返回 `Err(String)`。
pub trait HttpPort {
    fn post_json(&self, url: &str, bearer_token: &str, body: &str) -> Result<HttpResponse, String>;
}

/// 发送一条上报：请求体 `serde_json::to_string(payload)`，按 HTTP 状态分类结果。
pub fn send_report(
    http: &dyn HttpPort,
    cfg: &AppConfig,
    payload: &ReportPayload,
) -> Result<ReportOutcome, ReportError> {
    let body = serde_json::to_string(payload).map_err(|e| ReportError::Decode(e.to_string()))?;
    let url = cfg.report_url();
    let resp = http
        .post_json(&url, &cfg.api_token, &body)
        .map_err(ReportError::Network)?;
    Ok(classify(&resp))
}

/// HTTP 状态 → 结果分类。
fn classify(resp: &HttpResponse) -> ReportOutcome {
    match resp.status {
        200..=299 => ReportOutcome::Sent,
        400 => ReportOutcome::BadRequest,
        401 | 403 => ReportOutcome::Unauthorized,
        429 => ReportOutcome::RateLimited {
            retry_after_secs: retry_after_secs(resp),
        },
        _ => ReportOutcome::ServerError,
    }
}

/// 从 `Retry-After` 头解析秒数（大小写不敏感；解析失败返回 `None`）。
fn retry_after_secs(resp: &HttpResponse) -> Option<u64> {
    resp.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("retry-after"))
        .and_then(|(_, v)| v.trim().parse::<u64>().ok())
}

/// `ureq` 2.x 实现：POST + `Authorization: Bearer <token>` +
/// `Content-Type: application/json`，超时 10 秒。
#[derive(Debug, Clone, Copy, Default)]
pub struct UreqHttp;

impl HttpPort for UreqHttp {
    fn post_json(&self, url: &str, bearer_token: &str, body: &str) -> Result<HttpResponse, String> {
        // ureq 2.x 只开 `native-tls` feature 时默认 TLS 后端是 NoTlsConfig，
        // https 直接报 "cannot make HTTPS request because no TLS backend is
        // configured"；必须显式接入 native_tls::TlsConnector（Windows 走
        // schannel）。初始化失败时退回无 TLS 构造，http:// 仍可用。
        let mut builder = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(10));
        if let Ok(connector) = native_tls::TlsConnector::new() {
            builder = builder.tls_connector(std::sync::Arc::new(connector));
        }
        let agent = builder.build();
        let request = agent
            .post(url)
            .set("Authorization", &format!("Bearer {bearer_token}"))
            .set("Content-Type", "application/json");
        match request.send_string(body) {
            Ok(resp) => to_http_response(resp),
            Err(ureq::Error::Status(_status, resp)) => to_http_response(resp),
            Err(ureq::Error::Transport(t)) => Err(format!("transport error: {t}")),
        }
    }
}

fn to_http_response(resp: ureq::Response) -> Result<HttpResponse, String> {
    let status = resp.status();
    let mut headers = Vec::new();
    for name in resp.headers_names() {
        if let Some(value) = resp.header(&name) {
            headers.push((name, value.to_string()));
        }
    }
    let body = resp.into_string().map_err(|e| e.to_string())?;
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockHttp {
        result: Result<HttpResponse, String>,
        captured: std::sync::Mutex<Vec<(String, String, String)>>,
    }

    impl MockHttp {
        fn ok(status: u16, headers: Vec<(String, String)>) -> Self {
            MockHttp {
                result: Ok(HttpResponse {
                    status,
                    headers,
                    body: r#"{"code":0,"message":"ok","data":null}"#.to_string(),
                }),
                captured: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn failing(err: &str) -> Self {
            MockHttp {
                result: Err(err.to_string()),
                captured: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl HttpPort for MockHttp {
        fn post_json(
            &self,
            url: &str,
            bearer_token: &str,
            body: &str,
        ) -> Result<HttpResponse, String> {
            self.captured.lock().unwrap().push((
                url.to_string(),
                bearer_token.to_string(),
                body.to_string(),
            ));
            self.result.clone()
        }
    }

    fn test_config() -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.api_base_url = "http://127.0.0.1:8080".to_string();
        cfg.api_token = "tok-123".to_string();
        cfg
    }

    fn test_payload() -> ReportPayload {
        ReportPayload {
            device_id: "my-device".to_string(),
            device_name: "My Device".to_string(),
            status: crate::protocol::DeviceStatus::Active,
            timestamp: "2026-01-01T12:00:00Z".to_string(),
            app: None,
        }
    }

    #[test]
    fn status_2xx_is_sent() {
        let http = MockHttp::ok(200, vec![]);
        let out = send_report(&http, &test_config(), &test_payload()).unwrap();
        assert_eq!(out, ReportOutcome::Sent);
        let http = MockHttp::ok(204, vec![]);
        let out = send_report(&http, &test_config(), &test_payload()).unwrap();
        assert_eq!(out, ReportOutcome::Sent);
    }

    #[test]
    fn status_400_is_bad_request() {
        let http = MockHttp::ok(400, vec![]);
        let out = send_report(&http, &test_config(), &test_payload()).unwrap();
        assert_eq!(out, ReportOutcome::BadRequest);
    }

    #[test]
    fn status_401_and_403_are_unauthorized() {
        for status in [401, 403] {
            let http = MockHttp::ok(status, vec![]);
            let out = send_report(&http, &test_config(), &test_payload()).unwrap();
            assert_eq!(out, ReportOutcome::Unauthorized);
        }
    }

    #[test]
    fn status_429_with_retry_after_header() {
        let http = MockHttp::ok(429, vec![("Retry-After".to_string(), "7".to_string())]);
        let out = send_report(&http, &test_config(), &test_payload()).unwrap();
        assert_eq!(
            out,
            ReportOutcome::RateLimited {
                retry_after_secs: Some(7)
            }
        );
        // 头名大小写不敏感
        let http = MockHttp::ok(429, vec![("retry-after".to_string(), " 3 ".to_string())]);
        let out = send_report(&http, &test_config(), &test_payload()).unwrap();
        assert_eq!(
            out,
            ReportOutcome::RateLimited {
                retry_after_secs: Some(3)
            }
        );
    }

    #[test]
    fn status_429_without_or_bad_retry_after_header() {
        let http = MockHttp::ok(429, vec![]);
        let out = send_report(&http, &test_config(), &test_payload()).unwrap();
        assert_eq!(
            out,
            ReportOutcome::RateLimited {
                retry_after_secs: None
            }
        );
        let http = MockHttp::ok(429, vec![("Retry-After".to_string(), "soon".to_string())]);
        let out = send_report(&http, &test_config(), &test_payload()).unwrap();
        assert_eq!(
            out,
            ReportOutcome::RateLimited {
                retry_after_secs: None
            }
        );
    }

    #[test]
    fn status_5xx_and_others_are_server_error() {
        for status in [500, 502, 404, 418] {
            let http = MockHttp::ok(status, vec![]);
            let out = send_report(&http, &test_config(), &test_payload()).unwrap();
            assert_eq!(out, ReportOutcome::ServerError);
        }
    }

    #[test]
    fn transport_failure_is_network_error() {
        let http = MockHttp::failing("connection refused");
        let err = send_report(&http, &test_config(), &test_payload()).unwrap_err();
        assert!(matches!(err, ReportError::Network(_)));
        assert!(err.to_string().contains("connection refused"));
    }

    #[test]
    fn request_carries_url_token_and_json_body() {
        let http = MockHttp::ok(200, vec![]);
        let payload = test_payload();
        send_report(&http, &test_config(), &payload).unwrap();
        let captured = http.captured.lock().unwrap();
        assert_eq!(captured.len(), 1);
        let (url, token, body) = &captured[0];
        assert_eq!(url, "http://127.0.0.1:8080/api/v1/report");
        assert_eq!(token, "tok-123");
        let parsed: ReportPayload = serde_json::from_str(body).unwrap();
        assert_eq!(parsed, payload);
    }

    /// 回归测试：https 请求必须已接入 TLS 后端。
    /// ureq 2.x 仅开 `native-tls` feature 时默认 NoTlsConfig，对 https URL 会
    /// 快速报 "cannot make HTTPS request because no TLS backend is configured"；
    /// 接线后请求会真正发起 TCP 连接（本测试指向无人监听的 127.0.0.1:1，失败
    /// 文本是普通传输错误）。离线可用，不依赖网络。
    #[test]
    fn https_requests_have_tls_backend_wired() {
        let http = UreqHttp;
        let err = http
            .post_json("https://127.0.0.1:1/api/v1/report", "tok", "{}")
            .unwrap_err();
        assert!(
            !err.contains("no TLS backend"),
            "https TLS 后端未接线: {err}"
        );
    }
}
