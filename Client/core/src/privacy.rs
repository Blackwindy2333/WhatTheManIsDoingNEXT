//! 隐私策略（PLAN.md 阶段 1 / §10：标题默认不采集不上报、黑名单脱敏、隐私暂停）。

use crate::config::AppConfig;
use crate::protocol::{now_utc_iso, AppInfo, DeviceStatus, ReportPayload};

/// 名称是否命中黑名单：对黑名单项做大小写不敏感子串匹配；空白/空串项忽略。
pub fn is_blacklisted(cfg: &AppConfig, name: &str) -> bool {
    let name_l = name.to_lowercase();
    cfg.app_name_blacklist
        .iter()
        .map(|item| item.trim().to_lowercase())
        .filter(|item| !item.is_empty())
        .any(|item| name_l.contains(&item))
}

/// 按隐私规则组装上报体：
/// - `privacy_pause=true` → `status=paused` 且 `app=None`；
/// - `report_window_title=false` → `window_title=None`；
/// - 黑名单命中（`process_name`/`display_name`/`window_title` 任一）→
///   `process_name="redacted"`、`display_name=None`、`window_title=None`；
/// - `timestamp = now_utc_iso()`。
pub fn build_report(cfg: &AppConfig, app: Option<AppInfo>, status: DeviceStatus) -> ReportPayload {
    if cfg.privacy_pause {
        return ReportPayload {
            device_id: cfg.device_id.clone(),
            device_name: cfg.device_name.clone(),
            status: DeviceStatus::Paused,
            timestamp: now_utc_iso(),
            app: None,
        };
    }

    let app = app.map(|mut a| {
        let blacklisted = is_blacklisted(cfg, &a.process_name)
            || a
                .display_name
                .as_deref()
                .map(|d| is_blacklisted(cfg, d))
                .unwrap_or(false)
            || a
                .window_title
                .as_deref()
                .map(|t| is_blacklisted(cfg, t))
                .unwrap_or(false);
        if blacklisted {
            return AppInfo {
                process_name: "redacted".to_string(),
                display_name: None,
                window_title: None,
            };
        }
        if !cfg.report_window_title {
            a.window_title = None;
        }
        a
    });

    ReportPayload {
        device_id: cfg.device_id.clone(),
        device_name: cfg.device_name.clone(),
        status,
        timestamp: now_utc_iso(),
        app,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> AppInfo {
        AppInfo {
            process_name: "chrome.exe".to_string(),
            display_name: Some("Google Chrome".to_string()),
            window_title: Some("敏感页面 - Google Chrome".to_string()),
        }
    }

    #[test]
    fn window_title_dropped_by_default() {
        let cfg = AppConfig::default();
        assert!(!cfg.report_window_title);
        let payload = build_report(&cfg, Some(app()), DeviceStatus::Active);
        let a = payload.app.unwrap();
        assert_eq!(a.window_title, None);
        assert_eq!(a.process_name, "chrome.exe");
        assert_eq!(a.display_name.as_deref(), Some("Google Chrome"));
        assert_eq!(payload.status, DeviceStatus::Active);
    }

    #[test]
    fn window_title_carried_when_explicitly_enabled() {
        let mut cfg = AppConfig::default();
        cfg.report_window_title = true;
        let payload = build_report(&cfg, Some(app()), DeviceStatus::Active);
        let a = payload.app.unwrap();
        assert_eq!(a.window_title.as_deref(), Some("敏感页面 - Google Chrome"));
    }

    #[test]
    fn blacklist_redacts_process_name() {
        let mut cfg = AppConfig::default();
        cfg.report_window_title = true;
        cfg.app_name_blacklist = vec!["CHROME".to_string()];
        let payload = build_report(&cfg, Some(app()), DeviceStatus::Active);
        let a = payload.app.unwrap();
        assert_eq!(a.process_name, "redacted");
        assert_eq!(a.display_name, None);
        assert_eq!(a.window_title, None);
    }

    #[test]
    fn blacklist_matches_display_name_and_window_title() {
        let mut cfg = AppConfig::default();
        cfg.report_window_title = true;
        cfg.app_name_blacklist = vec!["google".to_string()];
        let payload = build_report(&cfg, Some(app()), DeviceStatus::Active);
        let a = payload.app.unwrap();
        assert_eq!(a.process_name, "redacted");

        let mut cfg2 = AppConfig::default();
        cfg2.report_window_title = true;
        cfg2.app_name_blacklist = vec!["敏感页面".to_string()];
        let payload2 = build_report(&cfg2, Some(app()), DeviceStatus::Active);
        let a2 = payload2.app.unwrap();
        assert_eq!(a2.process_name, "redacted");
    }

    #[test]
    fn privacy_pause_forces_paused_and_null_app() {
        let mut cfg = AppConfig::default();
        cfg.privacy_pause = true;
        let payload = build_report(&cfg, Some(app()), DeviceStatus::Active);
        assert_eq!(payload.status, DeviceStatus::Paused);
        assert_eq!(payload.app, None);
    }

    #[test]
    fn empty_blacklist_entries_ignored() {
        let mut cfg = AppConfig::default();
        cfg.app_name_blacklist = vec![String::new(), "   ".to_string()];
        assert!(!is_blacklisted(&cfg, "chrome.exe"));
        let payload = build_report(&cfg, Some(app()), DeviceStatus::Active);
        let a = payload.app.unwrap();
        assert_eq!(a.process_name, "chrome.exe");
    }

    #[test]
    fn is_blacklisted_substring_case_insensitive() {
        let mut cfg = AppConfig::default();
        cfg.app_name_blacklist = vec!["pass".to_string()];
        assert!(is_blacklisted(&cfg, "PasswordManager.exe"));
        assert!(is_blacklisted(&cfg, "MY-PASS-APP"));
        assert!(!is_blacklisted(&cfg, "chrome.exe"));
    }

    #[test]
    fn payload_carries_config_identity_and_timestamp() {
        let mut cfg = AppConfig::default();
        cfg.device_id = "pc-01".to_string();
        cfg.device_name = "书房电脑".to_string();
        let payload = build_report(&cfg, None, DeviceStatus::Idle);
        assert_eq!(payload.device_id, "pc-01");
        assert_eq!(payload.device_name, "书房电脑");
        assert_eq!(payload.status, DeviceStatus::Idle);
        assert!(payload.timestamp.ends_with('Z'));
    }
}
