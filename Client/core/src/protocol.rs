//! v1 上报协议与 envelope（PLAN.md §8：数据契约，v1 保持兼容）。
//!
//! POST /api/v1/report 的请求体为 [`ReportPayload`]，响应为统一 [`Envelope`]。

use serde::{Deserialize, Serialize};

/// 上报请求体。`timestamp` 为 UTC ISO-8601（`2026-01-01T12:00:00Z` 形态）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportPayload {
    pub device_id: String,
    pub device_name: String,
    pub status: DeviceStatus,
    pub timestamp: String,
    pub app: Option<AppInfo>,
}

/// 前台应用信息；`window_title` 仅在配置显式开启时携带（默认 `None`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppInfo {
    pub process_name: String,
    pub display_name: Option<String>,
    pub window_title: Option<String>,
}

/// 设备状态，序列化为小写字符串：`active|paused|idle|stopped`。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DeviceStatus {
    Active,
    Paused,
    Idle,
    Stopped,
}

impl DeviceStatus {
    /// 对应 v1 契约中的小写状态字符串。
    pub fn as_str(&self) -> &'static str {
        match self {
            DeviceStatus::Active => "active",
            DeviceStatus::Paused => "paused",
            DeviceStatus::Idle => "idle",
            DeviceStatus::Stopped => "stopped",
        }
    }
}

/// 统一响应信封：`{"code":0,"message":"ok","data":...}`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub code: u32,
    pub message: String,
    pub data: Option<T>,
}

/// 成功。
pub const CODE_OK: u32 = 0;
/// 请求体不合法。
pub const CODE_BAD_REQUEST: u32 = 40000;
/// token 缺失或不匹配。
pub const CODE_UNAUTHORIZED: u32 = 40100;
/// 无权限。
pub const CODE_FORBIDDEN: u32 = 40300;
/// 资源不存在。
pub const CODE_NOT_FOUND: u32 = 40400;
/// 触发限流。
pub const CODE_RATE_LIMITED: u32 = 42900;
/// 服务端内部错误。
pub const CODE_INTERNAL: u32 = 50000;

/// 当前 UTC 时间，格式 `%Y-%m-%dT%H:%M:%SZ`。
pub fn now_utc_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_app() -> AppInfo {
        AppInfo {
            process_name: "Code.exe".to_string(),
            display_name: Some("Visual Studio Code".to_string()),
            window_title: None,
        }
    }

    #[test]
    fn serde_round_trip_without_app() {
        let payload = ReportPayload {
            device_id: "my-device".to_string(),
            device_name: "My Device".to_string(),
            status: DeviceStatus::Active,
            timestamp: "2026-01-01T12:00:00Z".to_string(),
            app: None,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"app\":null"));
        let back: ReportPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back, payload);
    }

    #[test]
    fn serde_round_trip_with_app_and_null_window_title() {
        let payload = ReportPayload {
            device_id: "my-device".to_string(),
            device_name: "My Device".to_string(),
            status: DeviceStatus::Idle,
            timestamp: "2026-01-01T12:00:00Z".to_string(),
            app: Some(sample_app()),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"window_title\":null"));
        let back: ReportPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back, payload);
    }

    #[test]
    fn device_status_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&DeviceStatus::Active).unwrap(),
            "\"active\""
        );
        assert_eq!(
            serde_json::to_string(&DeviceStatus::Paused).unwrap(),
            "\"paused\""
        );
        assert_eq!(
            serde_json::to_string(&DeviceStatus::Idle).unwrap(),
            "\"idle\""
        );
        assert_eq!(
            serde_json::to_string(&DeviceStatus::Stopped).unwrap(),
            "\"stopped\""
        );
        let back: DeviceStatus = serde_json::from_str("\"paused\"").unwrap();
        assert_eq!(back, DeviceStatus::Paused);
        assert_eq!(DeviceStatus::Active.as_str(), "active");
        assert_eq!(DeviceStatus::Paused.as_str(), "paused");
        assert_eq!(DeviceStatus::Idle.as_str(), "idle");
        assert_eq!(DeviceStatus::Stopped.as_str(), "stopped");
    }

    #[test]
    fn now_utc_iso_shape() {
        let ts = now_utc_iso();
        assert!(ts.ends_with('Z'), "timestamp must end with Z: {ts}");
        assert_eq!(ts.len(), 20, "expected %Y-%m-%dT%H:%M:%SZ shape: {ts}");
        assert_eq!(&ts[10..11], "T");
        chrono::NaiveDateTime::parse_from_str(&ts, "%Y-%m-%dT%H:%M:%SZ")
            .unwrap_or_else(|e| panic!("timestamp {ts} not parseable: {e}"));
    }

    #[test]
    fn envelope_round_trip() {
        let env: Envelope<serde_json::Value> =
            serde_json::from_str(r#"{"code":0,"message":"ok","data":{"ok":true}}"#).unwrap();
        assert_eq!(env.code, CODE_OK);
        assert_eq!(env.message, "ok");
        assert!(env.data.is_some());
    }
}
