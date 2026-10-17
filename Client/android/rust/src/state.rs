//! 桥内纯逻辑（无 JNI、无线程，可宿主 `cargo test`）：
//! 节流/去重、状态映射、配置读写、状态 JSON、投递结果分类。
//!
//! 业务规则（隐私暂停、黑名单、窗口标题开关、显示名映射）全部由 wtmd-core
//! 提供，本模块只做 Android 侧的编排，不重复实现。

use std::time::{SystemTime, UNIX_EPOCH};

use wtmd_core::{
    build_report, now_utc_iso, resolve_display_name, AppConfig, AppInfo, DeviceStatus, ReportError,
    ReportOutcome, ReportPayload,
};

/// 前台事件去重/合并的最小间隔（ms）。Android 的 notificationTimeout≈300 对齐。
pub const THROTTLE_MS: u64 = 300;
/// 断网时上报缓存容量（PLAN §10：环形队列，恢复后报最新）。
pub const QUEUE_CAPACITY: usize = 100;
/// 指数退避基数与封顶。
pub const BACKOFF_BASE_MS: u64 = 1_000;
pub const BACKOFF_MAX_MS: u64 = 60_000;

/// 单调毫秒（挂钟；只用于节流/退避，容忍小幅跳变）。
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 前台事件按包名去重：包名为空或与上次相同 → 跳过。
pub fn is_duplicate_package(new_package: &str, last_package: Option<&str>) -> bool {
    let pkg = new_package.trim();
    pkg.is_empty() || last_package == Some(pkg)
}

/// 发送节流：相邻两次 HTTP 上报的最小间隔（事件风暴/多窗口时合并）。
#[derive(Debug, Clone)]
pub struct SendThrottle {
    min_interval_ms: u64,
    last_sent_ms: Option<u64>,
}

impl SendThrottle {
    pub fn new(min_interval_ms: u64) -> Self {
        SendThrottle {
            min_interval_ms,
            last_sent_ms: None,
        }
    }

    /// 现在是否允许发送。
    pub fn ready(&self, now_ms: u64) -> bool {
        match self.last_sent_ms {
            None => true,
            Some(last) => now_ms.saturating_sub(last) >= self.min_interval_ms,
        }
    }

    /// 距离允许下一次发送还剩多少毫秒（ready 时为 0）。
    pub fn remaining_ms(&self, now_ms: u64) -> u64 {
        match self.last_sent_ms {
            None => 0,
            Some(last) => self
                .min_interval_ms
                .saturating_sub(now_ms.saturating_sub(last)),
        }
    }

    /// 记录一次发送时刻。
    pub fn mark(&mut self, now_ms: u64) {
        self.last_sent_ms = Some(now_ms);
    }
}

/// 指数退避：第 n 次连续失败 → 1s × 2^(n-1)，封顶 60s。
pub fn backoff_ms(failed_attempts: u32) -> u64 {
    let exp = failed_attempts.saturating_sub(1).min(16);
    BACKOFF_BASE_MS.saturating_mul(1u64 << exp).min(BACKOFF_MAX_MS)
}

/// 前台事件 → AppInfo（业务规则归 wtmd-core：
/// process_name=包名；window_title 仅 `report_window_title` 开启时携带
/// （Android 事件只有 className，用它近似窗口标题）；display_name 走映射）。
pub fn build_foreground_app(cfg: &AppConfig, package_name: &str, class_name: &str) -> AppInfo {
    let pkg = package_name.trim();
    AppInfo {
        process_name: pkg.to_string(),
        display_name: Some(resolve_display_name(pkg, &cfg.display_name_map)),
        window_title: if cfg.report_window_title && !class_name.trim().is_empty() {
            Some(class_name.trim().to_string())
        } else {
            None
        },
    }
}

/// 采集状态（对外状态字符串取值 active / paused / stopped）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectState {
    Active,
    Paused,
    Stopped,
}

impl CollectState {
    pub fn as_str(&self) -> &'static str {
        match self {
            CollectState::Active => "active",
            CollectState::Paused => "paused",
            CollectState::Stopped => "stopped",
        }
    }
}

/// 状态切换上报体。paused / stopped 一律 app=None（PLAN §8）。
pub fn build_state_payload(
    cfg: &AppConfig,
    state: CollectState,
    current: Option<&AppInfo>,
) -> ReportPayload {
    match state {
        CollectState::Active => build_report(cfg, current.cloned(), DeviceStatus::Active),
        CollectState::Paused => build_report(cfg, None, DeviceStatus::Paused),
        CollectState::Stopped => {
            if cfg.privacy_pause {
                // 有意偏差（已在 README「偏差与取舍」记录）：wtmd-core 的
                // build_report 在 privacy_pause 下强制返回 paused+app=None；
                // 「停止采集」是用户显式动作，仍应上报 stopped，故此处直接构造
                // ReportPayload（app=None，timestamp 走 core 的 now_utc_iso）。
                ReportPayload {
                    device_id: cfg.device_id.clone(),
                    device_name: cfg.device_name.clone(),
                    status: DeviceStatus::Stopped,
                    timestamp: now_utc_iso(),
                    app: None,
                }
            } else {
                build_report(cfg, None, DeviceStatus::Stopped)
            }
        }
    }
}

/// 解析并校验配置 JSON（save_config_json 用）。
pub fn parse_config_json(json: &str) -> Result<AppConfig, String> {
    let cfg: AppConfig =
        serde_json::from_str(json).map_err(|e| format!("config JSON parse error: {e}"))?;
    cfg.validate().map_err(|e| format!("config invalid: {e}"))?;
    Ok(cfg)
}

/// 最近一次上报结果（供状态 JSON / 通知栏展示）。
#[derive(Debug, Clone)]
pub struct LastReport {
    /// sent / bad_request / unauthorized / rate_limited / server_error /
    /// network_error / decode_error。
    pub outcome: String,
    /// 上报时刻（UTC ISO-8601 Z）。
    pub at: String,
    pub error: Option<String>,
}

/// 单次投递决策（service 的上报线程按此驱动队列/退避）。
#[derive(Debug, Clone)]
pub enum Delivery {
    /// 已送达（2xx）：积压作废，退避清零。
    Sent,
    /// 永久失败（400/401/403/编码失败）：丢弃该载荷，不重试。
    Permanent { label: &'static str, error: String },
    /// 可重试（429/5xx/网络）：入环形缓存并按 Retry-After 或指数退避重试。
    Transient {
        label: &'static str,
        error: String,
        retry_after_secs: Option<u64>,
    },
}

/// core 的上报结果 → 投递决策 + 状态标签。
pub fn classify_delivery(result: Result<ReportOutcome, ReportError>) -> Delivery {
    match result {
        Ok(ReportOutcome::Sent) => Delivery::Sent,
        Ok(ReportOutcome::Unauthorized) => Delivery::Permanent {
            label: "unauthorized",
            error: "token missing or mismatched (401/403)".to_string(),
        },
        Ok(ReportOutcome::BadRequest) => Delivery::Permanent {
            label: "bad_request",
            error: "server rejected payload (400)".to_string(),
        },
        Ok(ReportOutcome::RateLimited { retry_after_secs }) => Delivery::Transient {
            label: "rate_limited",
            error: "server rate limited (429)".to_string(),
            retry_after_secs,
        },
        Ok(ReportOutcome::ServerError) => Delivery::Transient {
            label: "server_error",
            error: "server error (5xx or unexpected status)".to_string(),
            retry_after_secs: None,
        },
        Err(ReportError::Network(msg)) => Delivery::Transient {
            label: "network_error",
            error: msg,
            retry_after_secs: None,
        },
        Err(ReportError::Decode(msg)) => Delivery::Permanent {
            label: "decode_error",
            error: msg,
        },
    }
}

/// get_status_json 的输入快照。
pub struct StatusSnapshot<'a> {
    pub initialized: bool,
    pub state: CollectState,
    pub cfg: &'a AppConfig,
    pub current_app: Option<&'a AppInfo>,
    pub last_report: Option<&'a LastReport>,
    /// 基础设施错误（配置加载/保存等），与上报错误分开。
    pub last_error: Option<&'a str>,
    pub queue_len: usize,
}

/// 通知栏/状态区用 JSON：
/// {initialized,state,privacy_pause,report_window_title,current_app,last_report,
///  last_error,queue_len,api_base_url,device_id}
pub fn status_json(snap: &StatusSnapshot<'_>) -> String {
    use serde_json::json;

    let current_app = snap
        .current_app
        .map(serde_json::to_value)
        .and_then(|v| v.ok())
        .unwrap_or(serde_json::Value::Null);
    let last_report = snap
        .last_report
        .map(|r| {
            json!({
                "outcome": r.outcome,
                "at": r.at,
                "error": r.error,
            })
        })
        .unwrap_or(serde_json::Value::Null);

    json!({
        "initialized": snap.initialized,
        "state": snap.state.as_str(),
        "privacy_pause": snap.cfg.privacy_pause,
        "report_window_title": snap.cfg.report_window_title,
        "current_app": current_app,
        "last_report": last_report,
        "last_error": snap.last_error,
        "queue_len": snap.queue_len,
        "api_base_url": snap.cfg.api_base_url,
        "device_id": snap.cfg.device_id,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn cfg() -> AppConfig {
        AppConfig {
            api_base_url: "http://127.0.0.1:8080".to_string(),
            device_id: "dev-1".to_string(),
            device_name: "Dev One".to_string(),
            api_token: "tok".to_string(),
            poll_interval_ms: 1000,
            report_window_title: false,
            app_name_blacklist: vec!["secret".to_string()],
            privacy_pause: false,
            history_limit: 500,
            display_name_map: {
                let mut m = BTreeMap::new();
                m.insert("com.tencent.mm".to_string(), "微信".to_string());
                m
            },
        }
    }

    // ---- 节流/去重 ----

    #[test]
    fn dedup_by_package_name() {
        assert!(is_duplicate_package("", None));
        assert!(is_duplicate_package("   ", Some("a")));
        assert!(is_duplicate_package("a", Some("a")));
        assert!(is_duplicate_package(" a ", Some("a")));
        assert!(!is_duplicate_package("b", Some("a")));
        assert!(!is_duplicate_package("a", None));
    }

    #[test]
    fn throttle_respects_min_interval() {
        let mut t = SendThrottle::new(THROTTLE_MS);
        assert!(t.ready(1_000));
        t.mark(1_000);
        assert!(!t.ready(1_299));
        assert!(t.ready(1_300));
        assert!(t.ready(9_999));
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff_ms(0), BACKOFF_BASE_MS);
        assert_eq!(backoff_ms(1), 1_000);
        assert_eq!(backoff_ms(2), 2_000);
        assert_eq!(backoff_ms(3), 4_000);
        assert_eq!(backoff_ms(7), 60_000); // 1000<<6=64000 → 封顶 60s
        assert_eq!(backoff_ms(100), BACKOFF_MAX_MS);
    }

    // ---- 状态映射 ----

    #[test]
    fn foreground_app_respects_title_switch_and_map() {
        let mut c = cfg();
        let app = build_foreground_app(&c, "com.tencent.mm", "com.tencent.mm.ui.LauncherUI");
        assert_eq!(app.process_name, "com.tencent.mm");
        assert_eq!(app.display_name.as_deref(), Some("微信"));
        assert_eq!(app.window_title, None); // report_window_title=false

        c.report_window_title = true;
        let app = build_foreground_app(&c, "com.tencent.mm", "com.tencent.mm.ui.LauncherUI");
        assert_eq!(
            app.window_title.as_deref(),
            Some("com.tencent.mm.ui.LauncherUI")
        );

        // 未知包名：display_name 回落到原名，标题为空不携带
        let app = build_foreground_app(&c, "com.example.app", "  ");
        assert_eq!(app.display_name.as_deref(), Some("com.example.app"));
        assert_eq!(app.window_title, None);
    }

    #[test]
    fn paused_and_stopped_payloads_carry_no_app() {
        let c = cfg();
        let current = build_foreground_app(&c, "com.tencent.mm", "cls");
        let p = build_state_payload(&c, CollectState::Paused, Some(&current));
        assert_eq!(p.status, DeviceStatus::Paused);
        assert!(p.app.is_none());
        assert_eq!(p.device_id, "dev-1");

        let p = build_state_payload(&c, CollectState::Stopped, Some(&current));
        assert_eq!(p.status, DeviceStatus::Stopped);
        assert!(p.app.is_none());
    }

    #[test]
    fn stopped_still_reports_stopped_under_privacy_pause() {
        // 有意偏差的固化测试：privacy_pause 开启时 core 会把任何上报压成 paused，
        // 但「停止采集」必须上报 stopped。
        let mut c = cfg();
        c.privacy_pause = true;
        let p = build_state_payload(&c, CollectState::Stopped, None);
        assert_eq!(p.status, DeviceStatus::Stopped);
        assert!(p.app.is_none());

        // resume（Active）在 privacy_pause 下被 core 压成 paused+None
        let p = build_state_payload(&c, CollectState::Active, None);
        assert_eq!(p.status, DeviceStatus::Paused);
        assert!(p.app.is_none());
    }

    #[test]
    fn blacklist_redacts_app_in_report() {
        // 组装上报走 core：黑名单 → redacted（不在此重复实现，只验证链路）
        let c = cfg();
        let app = build_foreground_app(&c, "com.secret.app", "cls");
        let p = build_report(&c, Some(app), DeviceStatus::Active);
        let got = p.app.expect("app present");
        assert_eq!(got.process_name, "redacted");
        assert!(got.window_title.is_none());
    }

    // ---- 配置读写 ----

    #[test]
    fn config_json_round_trip() {
        let c = cfg();
        let json = serde_json::to_string_pretty(&c).unwrap();
        let back = parse_config_json(&json).expect("valid config");
        assert_eq!(back.device_id, "dev-1");
        assert_eq!(back.poll_interval_ms, 1000);
        assert_eq!(back.app_name_blacklist, vec!["secret".to_string()]);
        assert!(back.display_name_map.contains_key("com.tencent.mm"));
    }

    #[test]
    fn config_json_rejects_invalid() {
        assert!(parse_config_json("not json").is_err());
        // 非法 URL
        let bad = r#"{"api_base_url":"ftp://x","device_id":"a","device_name":"b","api_token":"c","poll_interval_ms":1000,"report_window_title":false,"app_name_blacklist":[],"privacy_pause":false,"history_limit":500,"display_name_map":{}}"#;
        let err = parse_config_json(bad).unwrap_err();
        assert!(err.contains("http"), "unexpected error: {err}");
        // 轮询间隔过小
        let bad = r#"{"api_base_url":"http://x","device_id":"a","device_name":"b","api_token":"c","poll_interval_ms":99,"report_window_title":false,"app_name_blacklist":[],"privacy_pause":false,"history_limit":500,"display_name_map":{}}"#;
        assert!(parse_config_json(bad).is_err());
        // device_id 空
        let bad = r#"{"api_base_url":"http://x","device_id":"  ","device_name":"b","api_token":"c","poll_interval_ms":100,"report_window_title":false,"app_name_blacklist":[],"privacy_pause":false,"history_limit":500,"display_name_map":{}}"#;
        assert!(parse_config_json(bad).is_err());
    }

    #[test]
    fn config_missing_fields_use_defaults() {
        // serde(default)：缺字段不报错（与 load_or_create 的宽容策略一致）
        let c = parse_config_json("{}").expect("defaults");
        assert_eq!(c.device_id, "my-device");
        assert!(!c.report_window_title);
        assert!(!c.privacy_pause);
    }

    // ---- 投递决策 ----

    #[test]
    fn classify_delivery_mapping() {
        assert!(matches!(
            classify_delivery(Ok(ReportOutcome::Sent)),
            Delivery::Sent
        ));
        match classify_delivery(Ok(ReportOutcome::Unauthorized)) {
            Delivery::Permanent { label, .. } => assert_eq!(label, "unauthorized"),
            other => panic!("unexpected: {other:?}"),
        }
        match classify_delivery(Ok(ReportOutcome::RateLimited {
            retry_after_secs: Some(30),
        })) {
            Delivery::Transient {
                label,
                retry_after_secs,
                ..
            } => {
                assert_eq!(label, "rate_limited");
                assert_eq!(retry_after_secs, Some(30));
            }
            other => panic!("unexpected: {other:?}"),
        }
        match classify_delivery(Err(ReportError::Network("boom".to_string()))) {
            Delivery::Transient { label, error, .. } => {
                assert_eq!(label, "network_error");
                assert_eq!(error, "boom");
            }
            other => panic!("unexpected: {other:?}"),
        }
        match classify_delivery(Err(ReportError::Decode("bad".to_string()))) {
            Delivery::Permanent { label, .. } => assert_eq!(label, "decode_error"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    // ---- 状态 JSON ----

    #[test]
    fn status_json_shape() {
        let c = cfg();
        let app = build_foreground_app(&c, "com.tencent.mm", "cls");
        let last = LastReport {
            outcome: "sent".to_string(),
            at: "2026-01-01T00:00:00Z".to_string(),
            error: None,
        };
        let s = status_json(&StatusSnapshot {
            initialized: true,
            state: CollectState::Active,
            cfg: &c,
            current_app: Some(&app),
            last_report: Some(&last),
            last_error: None,
            queue_len: 2,
        });
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["initialized"], true);
        assert_eq!(v["state"], "active");
        assert_eq!(v["privacy_pause"], false);
        assert_eq!(v["current_app"]["process_name"], "com.tencent.mm");
        assert_eq!(v["current_app"]["display_name"], "微信");
        assert_eq!(v["current_app"]["window_title"], serde_json::Value::Null);
        assert_eq!(v["last_report"]["outcome"], "sent");
        assert_eq!(v["queue_len"], 2);
        assert_eq!(v["device_id"], "dev-1");
    }

    #[test]
    fn status_json_without_app_or_report() {
        let c = cfg();
        let s = status_json(&StatusSnapshot {
            initialized: false,
            state: CollectState::Stopped,
            cfg: &c,
            current_app: None,
            last_report: None,
            last_error: Some("boom"),
            queue_len: 0,
        });
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["state"], "stopped");
        assert_eq!(v["current_app"], serde_json::Value::Null);
        assert_eq!(v["last_report"], serde_json::Value::Null);
        assert_eq!(v["last_error"], "boom");
    }
}
