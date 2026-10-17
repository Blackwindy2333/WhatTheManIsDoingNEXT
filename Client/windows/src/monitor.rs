//! 采集/上报状态机的纯逻辑部分（可单测）。
//!
//! Win32 采集在 `capture.rs`，上报队列与退避在 `reporter.rs`，
//! 本模块只做：分类、变化检测、心跳、退避计算、黑名单文本解析、状态文案。

use std::time::{Duration, Instant};

use wtmd_core::protocol::DeviceStatus;

/// 前台窗口采样结果（`capture::sample_foreground` 的产物）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FgSample {
    /// exe 文件名，如 `chrome.exe`；取不到时 `Unknown`。
    pub process_name: String,
    /// 窗口标题；空/取不到为 `None`（是否上报由 core 隐私规则决定）。
    pub window_title: Option<String>,
    /// Win32 窗口类名，用于识别桌面/锁屏。
    pub class_name: Option<String>,
}

/// 前台分类：有真实前台应用 = Active；桌面/锁屏/无窗口 = Idle。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FgState {
    Active,
    Idle,
}

/// 这些窗口类属于桌面/壁纸层，不算"前台应用"。
const IDLE_CLASSES: &[&str] = &["Progman", "WorkerW", "LockScreen"];

/// 锁屏进程（Win10/11 锁屏 UI）。
const IDLE_PROCESSES: &[&str] = &["LockApp.exe"];

/// 分类：`None`（无前台窗口）→ Idle；桌面/锁屏窗口 → Idle；否则 Active。
pub fn classify(sample: Option<&FgSample>) -> FgState {
    match sample {
        None => FgState::Idle,
        Some(s) => {
            let class = s.class_name.as_deref().unwrap_or("");
            if IDLE_CLASSES.iter().any(|c| class.eq_ignore_ascii_case(c)) {
                return FgState::Idle;
            }
            if IDLE_PROCESSES
                .iter()
                .any(|p| s.process_name.eq_ignore_ascii_case(p))
            {
                return FgState::Idle;
            }
            FgState::Active
        }
    }
}

/// 变化检测：进程名+标题与上次一致则不发（返回 false）。
///
/// 状态切换（Active↔Idle）也算变化，必须发一次；
/// Idle/无窗口时不比较进程名与标题（`app=None`）。
#[derive(Debug, Default)]
pub struct ChangeDetector {
    last: Option<(FgState, String, Option<String>)>,
}

impl ChangeDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// 采样并判断是否需要上报；内部记录最新状态。
    pub fn changed(&mut self, state: FgState, sample: Option<&FgSample>) -> bool {
        let key = match (state, sample) {
            (FgState::Active, Some(s)) => {
                (state, s.process_name.clone(), s.window_title.clone())
            }
            _ => (state, String::new(), None),
        };
        let changed = self.last.as_ref() != Some(&key);
        if changed {
            self.last = Some(key);
        }
        changed
    }

    /// 上次记录的状态键（测试/调试用）。
    #[allow(dead_code)]
    pub fn last_key(&self) -> Option<&(FgState, String, Option<String>)> {
        self.last.as_ref()
    }
}

/// 心跳补报间隔：窗口不变时也要周期性上报，避免服务端判离线。
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

/// 距上次事件（变化或心跳）是否已到心跳间隔。
pub fn heartbeat_due(last_event: Instant, now: Instant) -> bool {
    now.checked_duration_since(last_event)
        .is_some_and(|d| d >= HEARTBEAT_INTERVAL)
}

/// 网络/服务器错误的指数退避：5s、10s、20s、40s、80s、160s，封顶 300s。
pub fn backoff_after_failure(consecutive_failures: u32) -> Duration {
    let exp = consecutive_failures.saturating_sub(1).min(6);
    Duration::from_secs((5u64 << exp).min(300))
}

/// 429 未携带 Retry-After 时的缺省退避秒数。
pub const RATE_LIMIT_DEFAULT_SECS: u64 = 30;

/// 401/403（token 无效）的退避秒数：不重试死循环。
pub const UNAUTHORIZED_BACKOFF_SECS: u64 = 60;

/// 解析逗号分隔的进程黑名单文本。
///
/// 支持半/全角逗号、顿号、分号；去空白、去空项、保序去重。
pub fn parse_blacklist(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in text.split([',', '，', '、', ';', '；']) {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        if !out.iter().any(|x| x.eq_ignore_ascii_case(item)) {
            out.push(item.to_string());
        }
    }
    out
}

/// 序列化黑名单为设置框文本（逗号分隔）。
pub fn format_blacklist(items: &[String]) -> String {
    items.join(", ")
}

/// 状态 → 托盘展示文案。
pub fn status_text(status: DeviceStatus) -> &'static str {
    match status {
        DeviceStatus::Active => "采集中",
        DeviceStatus::Paused => "已暂停（隐私）",
        DeviceStatus::Idle => "空闲（桌面/锁屏）",
        DeviceStatus::Stopped => "已停止",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(process: &str, title: Option<&str>) -> FgSample {
        FgSample {
            process_name: process.to_string(),
            window_title: title.map(str::to_string),
            class_name: Some("Chrome_WidgetWin_1".to_string()),
        }
    }

    #[test]
    fn classify_none_is_idle() {
        assert_eq!(classify(None), FgState::Idle);
    }

    #[test]
    fn classify_desktop_and_lock_are_idle() {
        let desktop = FgSample {
            process_name: "explorer.exe".into(),
            window_title: None,
            class_name: Some("Progman".into()),
        };
        assert_eq!(classify(Some(&desktop)), FgState::Idle);
        let workerw = FgSample {
            class_name: Some("WorkerW".into()),
            ..desktop.clone()
        };
        assert_eq!(classify(Some(&workerw)), FgState::Idle);
        let lock = FgSample {
            process_name: "LockApp.exe".into(),
            window_title: None,
            class_name: Some("LockScreen".into()),
        };
        assert_eq!(classify(Some(&lock)), FgState::Idle);
    }

    #[test]
    fn classify_normal_app_is_active() {
        assert_eq!(classify(Some(&sample("chrome.exe", Some("GitHub")))), FgState::Active);
    }

    #[test]
    fn detector_same_process_and_title_not_reported() {
        let mut d = ChangeDetector::new();
        let s = sample("chrome.exe", Some("GitHub"));
        assert!(d.changed(FgState::Active, Some(&s)), "首次采样必须上报");
        assert!(!d.changed(FgState::Active, Some(&s)), "进程名+标题一致不发");
    }

    #[test]
    fn detector_title_change_reported() {
        let mut d = ChangeDetector::new();
        assert!(d.changed(FgState::Active, Some(&sample("chrome.exe", Some("A")))));
        assert!(d.changed(FgState::Active, Some(&sample("chrome.exe", Some("B")))));
    }

    #[test]
    fn detector_process_change_reported() {
        let mut d = ChangeDetector::new();
        assert!(d.changed(FgState::Active, Some(&sample("a.exe", Some("T")))));
        assert!(d.changed(FgState::Active, Some(&sample("b.exe", Some("T")))));
    }

    #[test]
    fn detector_title_none_vs_some_is_change() {
        let mut d = ChangeDetector::new();
        assert!(d.changed(FgState::Active, Some(&sample("a.exe", None))));
        assert!(d.changed(FgState::Active, Some(&sample("a.exe", Some("T")))));
        assert!(d.changed(FgState::Active, Some(&sample("a.exe", None))));
    }

    #[test]
    fn detector_state_switch_is_change() {
        let mut d = ChangeDetector::new();
        let s = sample("chrome.exe", Some("GitHub"));
        assert!(d.changed(FgState::Active, Some(&s)));
        assert!(d.changed(FgState::Idle, None), "Active→Idle 必须上报");
        assert!(!d.changed(FgState::Idle, None), "Idle 持续不重复发");
        assert!(d.changed(FgState::Active, Some(&s)), "Idle→Active 必须上报");
    }

    #[test]
    fn heartbeat_timing() {
        let t0 = Instant::now();
        assert!(!heartbeat_due(t0, t0 + Duration::from_secs(59)));
        assert!(heartbeat_due(t0, t0 + Duration::from_secs(60)));
        assert!(heartbeat_due(t0, t0 + Duration::from_secs(61)));
    }

    #[test]
    fn backoff_exponential_capped() {
        assert_eq!(backoff_after_failure(1), Duration::from_secs(5));
        assert_eq!(backoff_after_failure(2), Duration::from_secs(10));
        assert_eq!(backoff_after_failure(3), Duration::from_secs(20));
        assert_eq!(backoff_after_failure(4), Duration::from_secs(40));
        assert_eq!(backoff_after_failure(5), Duration::from_secs(80));
        assert_eq!(backoff_after_failure(6), Duration::from_secs(160));
        assert_eq!(backoff_after_failure(7), Duration::from_secs(300));
        assert_eq!(backoff_after_failure(100), Duration::from_secs(300));
        assert_eq!(backoff_after_failure(0), Duration::from_secs(5));
    }

    #[test]
    fn blacklist_parse_mixed_separators() {
        assert_eq!(
            parse_blacklist("chrome.exe, Code.exe，secret、 hidden；x;y"),
            vec!["chrome.exe", "Code.exe", "secret", "hidden", "x", "y"]
        );
    }

    #[test]
    fn blacklist_parse_trims_and_skips_empty() {
        assert_eq!(parse_blacklist("  a.exe ,,  ； ,b.exe  "), vec!["a.exe", "b.exe"]);
        assert!(parse_blacklist("").is_empty());
        assert!(parse_blacklist(" , ， ； ").is_empty());
    }

    #[test]
    fn blacklist_parse_dedup_case_insensitive_keeps_order() {
        assert_eq!(parse_blacklist("A.exe,a.exe,B.exe"), vec!["A.exe", "B.exe"]);
    }

    #[test]
    fn blacklist_roundtrip_format() {
        let items = parse_blacklist("a.exe, b.exe");
        assert_eq!(format_blacklist(&items), "a.exe, b.exe");
    }

    #[test]
    fn status_texts() {
        assert_eq!(status_text(DeviceStatus::Active), "采集中");
        assert_eq!(status_text(DeviceStatus::Stopped), "已停止");
    }
}
