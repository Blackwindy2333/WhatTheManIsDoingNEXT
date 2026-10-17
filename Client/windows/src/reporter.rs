//! 上报发送策略：队列缓存、退避、恢复补报（可单测，HTTP 经 `HttpPort` 注入）。

use std::time::{Duration, Instant};

use wtmd_core::config::AppConfig;
use wtmd_core::protocol::ReportPayload;
use wtmd_core::queue::ReportQueue;
use wtmd_core::report::{send_report, HttpPort, ReportOutcome};

use crate::monitor::{backoff_after_failure, RATE_LIMIT_DEFAULT_SECS, UNAUTHORIZED_BACKOFF_SECS};

/// 断网/服务器故障时的环形缓存容量（满丢最旧）。
pub const QUEUE_CAPACITY: usize = 100;

/// 一次发送的结果（含本地策略处理后的分类）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// 2xx，已送达。
    Sent,
    /// 401/403：token 无效，仅退避 60s，不入队。
    Unauthorized,
    /// 400：载荷被服务器拒绝，丢弃该条，不退避（下次变化再试）。
    BadRequest,
    /// 429：按 Retry-After（缺省 30s）退避，载荷入队。
    RateLimited,
    /// 网络失败/5xx：指数退避，载荷入队（满丢最旧）。
    FailedQueued,
}

/// 上报队列 + 退避状态。
pub struct Reporter {
    queue: ReportQueue,
    backoff_until: Option<Instant>,
    consecutive_failures: u32,
}

impl Default for Reporter {
    fn default() -> Self {
        Self::new()
    }
}

impl Reporter {
    pub fn new() -> Self {
        Self {
            queue: ReportQueue::new(QUEUE_CAPACITY),
            backoff_until: None,
            consecutive_failures: 0,
        }
    }

    /// 当前是否允许发起网络发送（退避窗口外）。
    pub fn can_send(&self, now: Instant) -> bool {
        self.backoff_until.is_none_or(|until| now >= until)
    }

    /// 距离退避结束的剩余时间（无退避返回 `None`）。
    #[allow(dead_code)]
    pub fn backoff_remaining(&self, now: Instant) -> Option<Duration> {
        self.backoff_until
            .and_then(|until| until.checked_duration_since(now))
    }

    pub fn queued_len(&self) -> usize {
        self.queue.len()
    }

    pub fn has_queued(&self) -> bool {
        !self.queue.is_empty()
    }

    /// 退避期间产生的上报直接入队（不发网络请求）。
    pub fn enqueue(&mut self, payload: ReportPayload) {
        self.queue.push(payload);
    }

    /// 发送一条上报并按结果更新退避/队列。调用方须先确认 `can_send`。
    pub fn submit(
        &mut self,
        http: &dyn HttpPort,
        cfg: &AppConfig,
        payload: ReportPayload,
        now: Instant,
    ) -> SubmitOutcome {
        match send_report(http, cfg, &payload) {
            Ok(ReportOutcome::Sent) => {
                self.consecutive_failures = 0;
                self.backoff_until = None;
                SubmitOutcome::Sent
            }
            Ok(ReportOutcome::Unauthorized) => {
                // token 无效：不重试死循环，固定退避 60s；不入队（避免无谓堆积）。
                self.backoff_until = Some(now + Duration::from_secs(UNAUTHORIZED_BACKOFF_SECS));
                SubmitOutcome::Unauthorized
            }
            Ok(ReportOutcome::BadRequest) => {
                // 载荷本身被拒，重发无意义：丢弃。
                SubmitOutcome::BadRequest
            }
            Ok(ReportOutcome::RateLimited { retry_after_secs }) => {
                self.queue.push(payload);
                let secs = retry_after_secs.unwrap_or(RATE_LIMIT_DEFAULT_SECS);
                self.backoff_until = Some(now + Duration::from_secs(secs));
                SubmitOutcome::RateLimited
            }
            Ok(ReportOutcome::ServerError) | Err(_) => {
                // 网络错误/5xx/解码错误：入队 + 指数退避。
                self.queue.push(payload);
                self.consecutive_failures += 1;
                self.backoff_until =
                    Some(now + backoff_after_failure(self.consecutive_failures));
                SubmitOutcome::FailedQueued
            }
        }
    }

    /// 恢复后补报队列最新一条（`pop_latest`，历史旧条被丢弃）。
    ///
    /// 队列为空返回 `None`；失败时该条会被 `submit` 重新入队。
    pub fn flush_latest(
        &mut self,
        http: &dyn HttpPort,
        cfg: &AppConfig,
        now: Instant,
    ) -> Option<SubmitOutcome> {
        let payload = self.queue.pop_latest()?;
        Some(self.submit(http, cfg, payload, now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    use wtmd_core::protocol::{AppInfo, DeviceStatus};
    use wtmd_core::report::HttpResponse;

    #[derive(Clone, Copy)]
    enum MockResp {
        Status(u16),
        Fail,
    }

    struct MockHttp {
        script: RefCell<VecDeque<MockResp>>,
        calls: Cell<u32>,
        headers: Vec<(String, String)>,
    }

    impl MockHttp {
        fn new(script: Vec<MockResp>) -> Self {
            Self {
                script: RefCell::new(script.into()),
                calls: Cell::new(0),
                headers: Vec::new(),
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
            self.calls.set(self.calls.get() + 1);
            match self.script.borrow_mut().pop_front() {
                Some(MockResp::Status(status)) => Ok(HttpResponse {
                    status,
                    headers: self.headers.clone(),
                    body: String::new(),
                }),
                Some(MockResp::Fail) => Err("network down".to_string()),
                None => Ok(HttpResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: String::new(),
                }),
            }
        }
    }

    fn cfg() -> AppConfig {
        AppConfig::default()
    }

    fn payload(tag: &str) -> ReportPayload {
        ReportPayload {
            device_id: "dev".to_string(),
            device_name: "Dev".to_string(),
            status: DeviceStatus::Active,
            timestamp: wtmd_core::protocol::now_utc_iso(),
            app: Some(AppInfo {
                process_name: tag.to_string(),
                display_name: None,
                window_title: None,
            }),
        }
    }

    #[test]
    fn success_resets_state() {
        let mut r = Reporter::new();
        let http = MockHttp::new(vec![MockResp::Status(200)]);
        let t0 = Instant::now();
        assert_eq!(r.submit(&http, &cfg(), payload("a"), t0), SubmitOutcome::Sent);
        assert!(r.can_send(t0));
        assert_eq!(r.queued_len(), 0);
    }

    #[test]
    fn network_failure_queues_and_backs_off_exponentially() {
        let mut r = Reporter::new();
        let http = MockHttp::new(vec![MockResp::Fail, MockResp::Fail]);
        let t0 = Instant::now();
        assert_eq!(
            r.submit(&http, &cfg(), payload("a"), t0),
            SubmitOutcome::FailedQueued
        );
        assert_eq!(r.queued_len(), 1);
        assert!(!r.can_send(t0), "失败后进入退避");
        assert_eq!(r.backoff_remaining(t0), Some(Duration::from_secs(5)));
        assert!(r.can_send(t0 + Duration::from_secs(5)));

        assert_eq!(
            r.submit(&http, &cfg(), payload("b"), t0 + Duration::from_secs(5)),
            SubmitOutcome::FailedQueued
        );
        assert_eq!(r.queued_len(), 2);
        assert_eq!(
            r.backoff_remaining(t0 + Duration::from_secs(5)),
            Some(Duration::from_secs(10)),
            "第 2 次失败退避 10s"
        );
    }

    #[test]
    fn rate_limited_uses_retry_after_and_queues() {
        let mut r = Reporter::new();
        let mut http = MockHttp::new(vec![MockResp::Status(429)]);
        http.headers
            .push(("Retry-After".to_string(), "7".to_string()));
        let t0 = Instant::now();
        assert_eq!(r.submit(&http, &cfg(), payload("a"), t0), SubmitOutcome::RateLimited);
        assert_eq!(r.queued_len(), 1);
        assert_eq!(r.backoff_remaining(t0), Some(Duration::from_secs(7)));
    }

    #[test]
    fn rate_limited_without_header_defaults_30s() {
        let mut r = Reporter::new();
        let http = MockHttp::new(vec![MockResp::Status(429)]);
        let t0 = Instant::now();
        assert_eq!(r.submit(&http, &cfg(), payload("a"), t0), SubmitOutcome::RateLimited);
        assert_eq!(r.backoff_remaining(t0), Some(Duration::from_secs(30)));
    }

    #[test]
    fn unauthorized_backs_off_60s_without_queue() {
        let mut r = Reporter::new();
        let http = MockHttp::new(vec![MockResp::Status(401)]);
        let t0 = Instant::now();
        assert_eq!(
            r.submit(&http, &cfg(), payload("a"), t0),
            SubmitOutcome::Unauthorized
        );
        assert_eq!(r.queued_len(), 0, "401 不入队");
        assert_eq!(r.backoff_remaining(t0), Some(Duration::from_secs(60)));
        // 403 同样处理
        let http = MockHttp::new(vec![MockResp::Status(403)]);
        assert_eq!(
            r.submit(&http, &cfg(), payload("a"), t0 + Duration::from_secs(61)),
            SubmitOutcome::Unauthorized
        );
    }

    #[test]
    fn bad_request_drops_payload_without_backoff() {
        let mut r = Reporter::new();
        let http = MockHttp::new(vec![MockResp::Status(400)]);
        let t0 = Instant::now();
        assert_eq!(r.submit(&http, &cfg(), payload("a"), t0), SubmitOutcome::BadRequest);
        assert_eq!(r.queued_len(), 0);
        assert!(r.can_send(t0), "400 不退避");
    }

    #[test]
    fn server_error_queues() {
        let mut r = Reporter::new();
        let http = MockHttp::new(vec![MockResp::Status(500)]);
        let t0 = Instant::now();
        assert_eq!(
            r.submit(&http, &cfg(), payload("a"), t0),
            SubmitOutcome::FailedQueued
        );
        assert_eq!(r.queued_len(), 1);
    }

    #[test]
    fn flush_latest_reports_newest_and_drops_older() {
        let mut r = Reporter::new();
        let failing = MockHttp::new(vec![
            MockResp::Fail,
            MockResp::Fail,
            MockResp::Fail,
        ]);
        let t0 = Instant::now();
        r.submit(&failing, &cfg(), payload("v1"), t0);
        r.submit(&failing, &cfg(), payload("v2"), t0 + Duration::from_secs(6));
        r.submit(&failing, &cfg(), payload("v3"), t0 + Duration::from_secs(20));
        assert_eq!(r.queued_len(), 3);

        let ok = MockHttp::new(vec![]);
        let t1 = t0 + Duration::from_secs(120);
        assert!(r.can_send(t1));
        let outcome = r.flush_latest(&ok, &cfg(), t1);
        assert_eq!(outcome, Some(SubmitOutcome::Sent));
        assert_eq!(r.queued_len(), 0, "pop_latest 清空队列只报最新一条");
        assert!(r.can_send(t1), "恢复后不再退避");
    }

    #[test]
    fn flush_latest_none_when_empty() {
        let mut r = Reporter::new();
        let http = MockHttp::new(vec![]);
        assert_eq!(r.flush_latest(&http, &cfg(), Instant::now()), None);
    }

    #[test]
    fn flush_latest_failure_requeues_single_latest() {
        let mut r = Reporter::new();
        // 三次失败：v1、v2 各占一次，flush_latest 的补报再占一次。
        let failing = MockHttp::new(vec![MockResp::Fail, MockResp::Fail, MockResp::Fail]);
        let t0 = Instant::now();
        r.submit(&failing, &cfg(), payload("v1"), t0);
        r.submit(&failing, &cfg(), payload("v2"), t0 + Duration::from_secs(10));
        assert_eq!(r.queued_len(), 2);
        let t1 = t0 + Duration::from_secs(120);
        let outcome = r.flush_latest(&failing, &cfg(), t1);
        assert_eq!(outcome, Some(SubmitOutcome::FailedQueued));
        assert_eq!(r.queued_len(), 1, "补报失败只保留最新一条");
        assert!(!r.can_send(t1), "失败后重新进入退避");
    }

    #[test]
    fn queue_capacity_100_drops_oldest() {
        let mut r = Reporter::new();
        for i in 0..150 {
            r.enqueue(payload(&format!("v{i}")));
        }
        assert_eq!(r.queued_len(), QUEUE_CAPACITY);
        let http = MockHttp::new(vec![]);
        let p = r.flush_latest(&http, &cfg(), Instant::now());
        assert_eq!(p, Some(SubmitOutcome::Sent));
        assert_eq!(r.queued_len(), 0);
    }

    #[test]
    fn deferral_enqueue_skips_network() {
        let mut r = Reporter::new();
        let http = MockHttp::new(vec![]);
        let t0 = Instant::now();
        r.submit(&http, &cfg(), payload("a"), t0); // 默认 200
        assert_eq!(http.calls.get(), 1);
        r.enqueue(payload("b"));
        assert_eq!(http.calls.get(), 1, "入队不触发网络请求");
    }
}
