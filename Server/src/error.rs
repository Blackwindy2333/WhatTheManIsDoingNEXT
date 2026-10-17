//! envelope v1：成功 `{"code":0,"message":"ok","data":...}`，失败 `{"code":<c>,"message":...,"data":null}`。

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::{json, Value};

pub const CODE_OK: i64 = 0;
pub const CODE_BAD_REQUEST: i64 = 40000;
pub const CODE_UNAUTHORIZED: i64 = 40100;
pub const CODE_FORBIDDEN: i64 = 40300;
pub const CODE_NOT_FOUND: i64 = 40400;
pub const CODE_RATE_LIMITED: i64 = 42900;
pub const CODE_INTERNAL: i64 = 50000;

/// 成功 envelope。
pub fn ok<T: Serialize>(data: T) -> Json<Value> {
    Json(json!({ "code": CODE_OK, "message": "ok", "data": data }))
}

/// 带错误码的 API 错误；`IntoResponse` 时输出失败 envelope。
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: i64,
    pub message: String,
    pub headers: Vec<(HeaderName, HeaderValue)>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: i64, message: impl Into<String>) -> Self {
        ApiError {
            status,
            code,
            message: message.into(),
            headers: Vec::new(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, CODE_BAD_REQUEST, message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, CODE_UNAUTHORIZED, message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, CODE_FORBIDDEN, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, CODE_NOT_FOUND, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, CODE_INTERNAL, message)
    }

    /// 429 + Retry-After / X-RateLimit-* 头。
    pub fn rate_limited(retry_after: u64, limit: u64, remaining: u64, reset: u64) -> Self {
        let mut err = Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            CODE_RATE_LIMITED,
            "rate limit exceeded, try again later",
        );
        err.push_header("retry-after", &retry_after.to_string());
        err.push_header("x-ratelimit-limit", &limit.to_string());
        err.push_header("x-ratelimit-remaining", &remaining.to_string());
        err.push_header("x-ratelimit-reset", &reset.to_string());
        err
    }

    fn push_header(&mut self, name: &'static str, value: &str) {
        if let Ok(value) = HeaderValue::from_str(value) {
            self.headers.push((HeaderName::from_static(name), value));
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({
            "code": self.code,
            "message": self.message,
            "data": Value::Null,
        });
        let mut response = (self.status, Json(body)).into_response();
        for (name, value) in self.headers {
            response.headers_mut().insert(name, value);
        }
        response
    }
}

/// 构建应用时的错误（数据库打开/初始化）。
#[derive(Debug)]
pub enum BuildError {
    Db(rusqlite::Error),
    Io(std::io::Error),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::Db(e) => write!(f, "database error: {e}"),
            BuildError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for BuildError {}

impl From<rusqlite::Error> for BuildError {
    fn from(value: rusqlite::Error) -> Self {
        BuildError::Db(value)
    }
}

impl From<std::io::Error> for BuildError {
    fn from(value: std::io::Error) -> Self {
        BuildError::Io(value)
    }
}
