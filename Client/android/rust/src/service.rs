//! 上报服务：全局共享状态 + 串行上报线程。
//!
//! 语义（PLAN §10 / 任务书）：
//! - 事件风暴：`pending` 只保留最新一条（最新覆盖合并），HTTP 发送有 300ms 节流；
//! - 断网：失败载荷进 `ReportQueue(100)` 环形缓存，指数退避（1s→60s）；
//! - 429：按 `Retry-After` 退避（无该头则指数退避）；
//! - 恢复后：无新事件时 `pop_latest()` 补报缓存中最新一条，成功即作废旧积压；
//! - 400/401/403/编码失败：永久失败，丢弃载荷不重试（避免打爆服务端）。

use std::path::PathBuf;
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use wtmd_core::report::send_report;
use wtmd_core::{
    build_report, now_utc_iso, AppConfig, AppInfo, DeviceStatus, ReportPayload, ReportQueue,
    UreqHttp,
};

use crate::state::{
    backoff_ms, build_foreground_app, build_state_payload, classify_delivery, now_ms,
    parse_config_json, status_json, CollectState, Delivery, LastReport, SendThrottle, StatusSnapshot,
    QUEUE_CAPACITY, THROTTLE_MS,
};

/// 取下一条待发载荷：新事件（pending）优先，其次缓存最新（pop_latest 顺带清空其余）。
pub fn take_next_payload(
    pending: &mut Option<ReportPayload>,
    queue: &mut ReportQueue,
) -> Option<ReportPayload> {
    pending.take().or_else(|| queue.pop_latest())
}

/// 投递决策 → 队列/重试计数状态迁移（上报线程与单测共用）。
pub fn apply_delivery(
    queue: &mut ReportQueue,
    failed_attempts: &mut u32,
    delivery: Delivery,
    payload: ReportPayload,
) {
    match delivery {
        Delivery::Sent => {
            // 最新载荷已送达：旧积压作废（pop_latest 清空队列）
            *failed_attempts = 0;
            let _ = queue.pop_latest();
        }
        Delivery::Permanent { .. } => {
            // 永久失败：丢弃该载荷，不重试
            *failed_attempts = 0;
        }
        Delivery::Transient { .. } => {
            // 可重试：入环形缓存（满则丢最旧），累计失败次数驱动退避
            *failed_attempts += 1;
            queue.push(payload);
        }
    }
}

struct Inner {
    cfg: AppConfig,
    cfg_path: PathBuf,
    stopped: bool,
    current_app: Option<AppInfo>,
    last_pkg: Option<String>,
    last_class: Option<String>,
    pending: Option<ReportPayload>,
    queue: ReportQueue,
    last_report: Option<LastReport>,
    last_error: Option<String>,
    failed_attempts: u32,
}

struct Shared {
    state: Mutex<Option<Inner>>,
    cv: Condvar,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

fn shared() -> &'static Shared {
    SHARED.get_or_init(|| Shared {
        state: Mutex::new(None),
        cv: Condvar::new(),
    })
}

/// 锁辅助：panic 中毒也照常取回数据（跨 FFI 语境下不因毒锁二次 panic）。
fn lock_state(sh: &Shared) -> MutexGuard<'_, Option<Inner>> {
    sh.state.lock().unwrap_or_else(|e| e.into_inner())
}

fn state_of<'a>(guard: &'a mut Option<Inner>) -> Option<&'a mut Inner> {
    guard.as_mut()
}

/// 初始化：load_or_create 配置并启动上报线程。幂等（重复调用返回 true）。
///
/// 返回 false 表示配置加载有问题（回退默认配置、错误记入状态 JSON）
/// 或上报线程启动失败；服务仍然可用。
pub fn init(config_path: &str) -> bool {
    let sh = shared();
    let mut guard = lock_state(sh);
    if guard.is_some() {
        return true;
    }

    let path = PathBuf::from(config_path);
    let (cfg, load_error) = match AppConfig::load_or_create(&path) {
        Ok(c) => (c, None),
        Err(e) => (
            AppConfig::default(),
            Some(format!("config load failed: {e}")),
        ),
    };

    *guard = Some(Inner {
        cfg,
        cfg_path: path,
        stopped: false,
        current_app: None,
        last_pkg: None,
        last_class: None,
        pending: None,
        queue: ReportQueue::new(QUEUE_CAPACITY),
        last_report: None,
        last_error: load_error.clone(),
        failed_attempts: 0,
    });
    drop(guard);

    let spawned = std::thread::Builder::new()
        .name("wtmd-reporter".to_string())
        .spawn(reporter_loop)
        .is_ok();
    if !spawned {
        let mut guard = lock_state(sh);
        if let Some(inner) = state_of(&mut guard) {
            inner.last_error = Some("failed to start reporter thread".to_string());
        }
        return false;
    }
    load_error.is_none()
}

/// 前台应用变化：按包名去重后构造 AppInfo → build_report(Active) → 最新覆盖入队。
pub fn on_foreground_change(package_name: &str, class_name: &str) {
    let sh = shared();
    let mut guard = lock_state(sh);
    let Some(inner) = state_of(&mut guard) else {
        return;
    };
    if inner.stopped {
        return; // 停止采集：不消费事件
    }
    if crate::state::is_duplicate_package(package_name, inner.last_pkg.as_deref()) {
        return;
    }

    let app = build_foreground_app(&inner.cfg, package_name, class_name);
    let payload = build_report(&inner.cfg, Some(app.clone()), DeviceStatus::Active);
    inner.current_app = Some(app);
    inner.last_pkg = Some(package_name.trim().to_string());
    inner.last_class = Some(class_name.trim().to_string());
    inner.pending = Some(payload); // 最新覆盖：事件风暴合并
    sh.cv.notify_all();
}

/// 隐私暂停开关：持久化配置并上报状态切换（paused 时 app=None）。
pub fn set_paused(paused: bool) {
    let sh = shared();
    let mut guard = lock_state(sh);
    let Some(inner) = state_of(&mut guard) else {
        return;
    };
    inner.stopped = false;
    inner.cfg.privacy_pause = paused;
    if let Err(e) = inner.cfg.save(&inner.cfg_path) {
        inner.last_error = Some(format!("config save failed: {e}"));
    } else {
        inner.last_error = None;
    }
    let state = if paused {
        CollectState::Paused
    } else {
        CollectState::Active
    };
    let payload = build_state_payload(&inner.cfg, state, inner.current_app.as_ref());
    inner.pending = Some(payload);
    sh.cv.notify_all();
}

/// 停止采集：上报 stopped（app=None），此后忽略前台事件。
pub fn set_stopped() {
    let sh = shared();
    let mut guard = lock_state(sh);
    let Some(inner) = state_of(&mut guard) else {
        return;
    };
    inner.stopped = true;
    let payload = build_state_payload(&inner.cfg, CollectState::Stopped, inner.current_app.as_ref());
    inner.pending = Some(payload);
    sh.cv.notify_all();
}

/// 通知栏/状态区 JSON（见 state::status_json）。
pub fn get_status_json() -> String {
    let sh = shared();
    let guard = lock_state(sh);
    let fallback_cfg = AppConfig::default();
    let Some(inner) = guard.as_ref() else {
        return status_json(&StatusSnapshot {
            initialized: false,
            state: CollectState::Stopped,
            cfg: &fallback_cfg,
            current_app: None,
            last_report: None,
            last_error: Some("bridge not initialized"),
            queue_len: 0,
        });
    };
    let state = if inner.stopped {
        CollectState::Stopped
    } else if inner.cfg.privacy_pause {
        CollectState::Paused
    } else {
        CollectState::Active
    };
    status_json(&StatusSnapshot {
        initialized: true,
        state,
        cfg: &inner.cfg,
        current_app: inner.current_app.as_ref(),
        last_report: inner.last_report.as_ref(),
        last_error: inner.last_error.as_deref(),
        queue_len: inner.queue.len(),
    })
}

/// 设置界面读配置（pretty JSON）。
pub fn get_config_json() -> String {
    let sh = shared();
    let guard = lock_state(sh);
    match guard.as_ref() {
        Some(inner) => {
            serde_json::to_string_pretty(&inner.cfg).unwrap_or_else(|_| "{}".to_string())
        }
        None => {
            let fallback = AppConfig::default();
            serde_json::to_string_pretty(&fallback).unwrap_or_else(|_| "{}".to_string())
        }
    }
}

/// 设置界面写配置：解析+校验+持久化+热应用。成功 true。
pub fn save_config_json(json: &str) -> bool {
    let sh = shared();
    let mut guard = lock_state(sh);
    let Some(inner) = state_of(&mut guard) else {
        return false;
    };
    let cfg = match parse_config_json(json) {
        Ok(c) => c,
        Err(e) => {
            inner.last_error = Some(e);
            return false;
        }
    };
    if let Err(e) = cfg.save(&inner.cfg_path) {
        inner.last_error = Some(format!("config save failed: {e}"));
        return false;
    }
    inner.cfg = cfg;
    // 热应用：显示名映射/黑名单/标题开关可能变化，按新配置重建当前快照
    if let Some(pkg) = inner.last_pkg.clone() {
        let class = inner.last_class.clone().unwrap_or_default();
        inner.current_app = Some(build_foreground_app(&inner.cfg, &pkg, &class));
    }
    inner.last_error = None;
    true
}

/// 串行上报线程主体。
fn reporter_loop() {
    let sh = shared();
    let http = UreqHttp;
    let mut throttle = SendThrottle::new(THROTTLE_MS);
    let mut next_attempt_ms: u64 = 0;

    loop {
        // 1) 等待「有活 + 节流/退避允许」
        let mut guard = lock_state(sh);
        loop {
            let Some(inner) = state_of(&mut guard) else {
                return; // init 后不会回退，防御性返回
            };
            let now = now_ms();
            let has_work = inner.pending.is_some() || !inner.queue.is_empty();
            if has_work && throttle.ready(now) && now >= next_attempt_ms {
                break;
            }
            let wait_ms = if has_work {
                throttle
                    .remaining_ms(now)
                    .max(next_attempt_ms.saturating_sub(now))
                    .clamp(1, 1_000)
            } else {
                1_000 // 空闲兜底唤醒；事件到达由 notify 唤醒
            };
            guard = sh
                .cv
                .wait_timeout(guard, Duration::from_millis(wait_ms))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }

        // 2) 取最新一条（pending 优先，其次缓存 pop_latest）
        let Some(inner) = state_of(&mut guard) else {
            return;
        };
        let Some(payload) = take_next_payload(&mut inner.pending, &mut inner.queue) else {
            continue;
        };
        let cfg = inner.cfg.clone();
        drop(guard); // 网络 IO（≤10s）期间不持锁

        // 3) 发送 + 分类
        let result = send_report(&http, &cfg, &payload);
        let delivery = classify_delivery(result);

        // 4) 回写状态
        let mut guard = lock_state(sh);
        let Some(inner) = state_of(&mut guard) else {
            return;
        };
        throttle.mark(now_ms());
        let (label, error) = match &delivery {
            Delivery::Sent => ("sent", None),
            Delivery::Permanent { label, error } => (*label, Some(error.clone())),
            Delivery::Transient { label, error, .. } => (*label, Some(error.clone())),
        };
        match &delivery {
            Delivery::Transient { retry_after_secs, .. } => {
                let delay = retry_after_secs
                    .map(|s| s.saturating_mul(1_000))
                    .unwrap_or_else(|| backoff_ms(inner.failed_attempts + 1));
                next_attempt_ms = now_ms().saturating_add(delay);
            }
            _ => next_attempt_ms = 0,
        }
        apply_delivery(&mut inner.queue, &mut inner.failed_attempts, delivery, payload);
        inner.last_report = Some(LastReport {
            outcome: label.to_string(),
            at: now_utc_iso(),
            error,
        });
        sh.cv.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use wtmd_core::{HttpPort, HttpResponse, ReportError, ReportOutcome};

    /// 可注入 mock：按队列吐响应/失败，空则默认 200。
    struct MockHttp {
        responses: RefCell<VecDeque<Result<HttpResponse, String>>>,
    }

    impl MockHttp {
        fn new(responses: Vec<Result<HttpResponse, String>>) -> Self {
            MockHttp {
                responses: RefCell::new(responses.into()),
            }
        }
    }

    impl HttpPort for MockHttp {
        fn post_json(
            &self,
            _url: &str,
            _bearer_token: &str,
            _body: &str,
        ) -> Result<HttpResponse, String> {
            self.responses
                .borrow_mut()
                .pop_front()
                .unwrap_or(Ok(HttpResponse {
                    status: 200,
                    headers: vec![],
                    body: "{}".to_string(),
                }))
        }
    }

    fn resp(status: u16) -> Result<HttpResponse, String> {
        Ok(HttpResponse {
            status,
            headers: vec![],
            body: "{}".to_string(),
        })
    }

    fn resp_retry_after(status: u16, secs: u64) -> Result<HttpResponse, String> {
        Ok(HttpResponse {
            status,
            headers: vec![("Retry-After".to_string(), secs.to_string())],
            body: "{}".to_string(),
        })
    }

    fn cfg() -> AppConfig {
        AppConfig {
            api_base_url: "http://127.0.0.1:8080".to_string(),
            device_id: "dev-1".to_string(),
            device_name: "Dev One".to_string(),
            api_token: "tok".to_string(),
            ..AppConfig::default()
        }
    }

    fn payload_of(cfg: &AppConfig, pkg: &str) -> ReportPayload {
        let app = build_foreground_app(cfg, pkg, "cls");
        build_report(cfg, Some(app), DeviceStatus::Active)
    }

    #[test]
    fn offline_backlog_reports_latest_on_recovery() {
        let cfg = cfg();
        let mut queue = ReportQueue::new(QUEUE_CAPACITY);
        let mut attempts = 0u32;

        // 断网：两条前台事件都失败 → 入缓存
        let failing = MockHttp::new(vec![Err("net down".to_string()), Err("net down".to_string())]);
        for pkg in ["com.a", "com.b"] {
            let payload = payload_of(&cfg, pkg);
            let delivery = classify_delivery(send_report(&failing, &cfg, &payload));
            apply_delivery(&mut queue, &mut attempts, delivery, payload);
        }
        assert_eq!(queue.len(), 2);
        assert_eq!(attempts, 2);
        // 2 次连续失败后，下次重试延迟 = backoff_ms(attempts + 1) = 4s（线程内语义）
        assert_eq!(backoff_ms(attempts + 1), 4_000);

        // 恢复：无新事件时 pop_latest 补报最新（com.b），成功后积压作废
        let next = take_next_payload(&mut None, &mut queue).expect("backlog");
        assert_eq!(next.app.as_ref().unwrap().process_name, "com.b");
        let ok = MockHttp::new(vec![]);
        let delivery = classify_delivery(send_report(&ok, &cfg, &next));
        apply_delivery(&mut queue, &mut attempts, delivery, next);
        assert!(queue.is_empty());
        assert_eq!(attempts, 0);
    }

    #[test]
    fn pending_wins_over_backlog() {
        let cfg = cfg();
        let mut queue = ReportQueue::new(QUEUE_CAPACITY);
        queue.push(payload_of(&cfg, "com.old"));
        let mut pending = Some(payload_of(&cfg, "com.newest"));
        let got = take_next_payload(&mut pending, &mut queue).expect("work");
        assert_eq!(got.app.as_ref().unwrap().process_name, "com.newest");
        assert!(pending.is_none());
    }

    #[test]
    fn permanent_failure_drops_payload_without_retry() {
        let cfg = cfg();
        let mut queue = ReportQueue::new(QUEUE_CAPACITY);
        let mut attempts = 0u32;
        let unauthorized = MockHttp::new(vec![resp(401)]);
        let payload = payload_of(&cfg, "com.a");
        let delivery = classify_delivery(send_report(&unauthorized, &cfg, &payload));
        assert!(matches!(
            delivery,
            Delivery::Permanent { label: "unauthorized", .. }
        ));
        apply_delivery(&mut queue, &mut attempts, delivery, payload);
        assert!(queue.is_empty());
        assert_eq!(attempts, 0);
    }

    #[test]
    fn rate_limited_carries_retry_after_and_queues() {
        let cfg = cfg();
        let mut queue = ReportQueue::new(QUEUE_CAPACITY);
        let mut attempts = 0u32;
        let limited = MockHttp::new(vec![resp_retry_after(429, 30)]);
        let payload = payload_of(&cfg, "com.a");
        let delivery = classify_delivery(send_report(&limited, &cfg, &payload));
        match &delivery {
            Delivery::Transient {
                label,
                retry_after_secs,
                ..
            } => {
                assert_eq!(*label, "rate_limited");
                assert_eq!(*retry_after_secs, Some(30));
            }
            other => panic!("unexpected: {other:?}"),
        }
        apply_delivery(&mut queue, &mut attempts, delivery, payload);
        assert_eq!(queue.len(), 1);
        assert_eq!(attempts, 1);
    }

    #[test]
    fn network_error_is_transient() {
        let cfg = cfg();
        let failing = MockHttp::new(vec![Err("boom".to_string())]);
        let payload = payload_of(&cfg, "com.a");
        let delivery = classify_delivery(send_report(&failing, &cfg, &payload));
        match delivery {
            Delivery::Transient { label, error, .. } => {
                assert_eq!(label, "network_error");
                assert_eq!(error, "boom");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn outcome_types_are_constructible() {
        // 编译期锚定 core 类型面（防止 core 改签名时漏改桥）
        let o: ReportOutcome = ReportOutcome::Sent;
        let e: ReportError = ReportError::Decode("x".to_string());
        assert!(matches!(o, ReportOutcome::Sent));
        assert!(matches!(e, ReportError::Decode(_)));
    }
}
