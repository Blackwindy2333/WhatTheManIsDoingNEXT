//! 按客户端 IP 的滑动窗口限流（每分钟请求数）。

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const WINDOW_SECONDS: f64 = 60.0;

/// 一次限流判定的结果（含响应头所需数值）。
#[derive(Debug, Clone, Copy)]
pub struct RateDecision {
    pub allowed: bool,
    pub limit: u64,
    pub remaining: u64,
    /// 秒；被拒时为建议重试等待，允许时为 0。
    pub retry_after: u64,
    /// 窗口释放时刻（epoch 秒）。
    pub reset: u64,
}

/// 每个 key（客户端 IP）独立的滑动窗口。
pub struct RateLimiter {
    limit: u64,
    entries: Mutex<HashMap<String, VecDeque<f64>>>,
}

impl RateLimiter {
    pub fn new(limit_per_minute: i64) -> Self {
        RateLimiter {
            limit: limit_per_minute.max(1) as u64,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }

    fn now() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }

    /// 记录一次请求并返回判定（与原 Python `GlobalRateLimiter.check` 语义一致）。
    pub fn check(&self, key: &str) -> RateDecision {
        let now = Self::now();
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let stamps = entries.entry(key.to_string()).or_default();
        let cutoff = now - WINDOW_SECONDS;
        while stamps.front().is_some_and(|t| *t < cutoff) {
            stamps.pop_front();
        }
        let limit = self.limit;
        let used = stamps.len() as u64;
        if used < limit {
            stamps.push_back(now);
            let oldest = stamps.front().copied().unwrap_or(now);
            return RateDecision {
                allowed: true,
                limit,
                remaining: limit - used - 1,
                retry_after: 0,
                reset: (oldest + WINDOW_SECONDS).ceil().max(now.ceil()) as u64,
            };
        }
        let oldest = stamps.front().copied().unwrap_or(now);
        RateDecision {
            allowed: false,
            limit,
            remaining: 0,
            retry_after: ((oldest + WINDOW_SECONDS - now).round() as u64).max(1),
            reset: (oldest + WINDOW_SECONDS).ceil() as u64,
        }
    }
}
