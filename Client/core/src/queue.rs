//! 离线上报队列（PLAN.md 阶段 4：断网环形队列缓存，恢复后报最新）。

use crate::protocol::ReportPayload;
use std::collections::VecDeque;

/// 环形队列：满则丢最旧；`pop_latest` 清空队列只返回最新一条。
#[derive(Debug, Clone)]
pub struct ReportQueue {
    items: VecDeque<ReportPayload>,
    capacity: usize,
}

impl ReportQueue {
    /// 新建队列；`capacity` 为 0 时按 1 处理。
    pub fn new(capacity: usize) -> Self {
        ReportQueue {
            items: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    /// 入队；队列满时丢弃最旧的一条。
    pub fn push(&mut self, payload: ReportPayload) {
        if self.items.len() >= self.capacity {
            self.items.pop_front();
        }
        self.items.push_back(payload);
    }

    /// 当前条数。
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 清空队列，只返回最新一条（“断网缓存，恢复后报最新”）。
    pub fn pop_latest(&mut self) -> Option<ReportPayload> {
        let latest = self.items.pop_back();
        self.items.clear();
        latest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{AppInfo, DeviceStatus};

    fn payload(tag: &str) -> ReportPayload {
        ReportPayload {
            device_id: "my-device".to_string(),
            device_name: "My Device".to_string(),
            status: DeviceStatus::Active,
            timestamp: "2026-01-01T12:00:00Z".to_string(),
            app: Some(AppInfo {
                process_name: tag.to_string(),
                display_name: None,
                window_title: None,
            }),
        }
    }

    fn tag_of(p: &ReportPayload) -> String {
        p.app.as_ref().unwrap().process_name.clone()
    }

    #[test]
    fn overflow_drops_oldest() {
        let mut q = ReportQueue::new(2);
        q.push(payload("a"));
        q.push(payload("b"));
        q.push(payload("c"));
        assert_eq!(q.len(), 2);
        let latest = q.pop_latest().unwrap();
        assert_eq!(tag_of(&latest), "c");
        assert!(q.is_empty());
    }

    #[test]
    fn pop_latest_clears_and_returns_newest() {
        let mut q = ReportQueue::new(10);
        assert!(q.pop_latest().is_none());
        q.push(payload("a"));
        q.push(payload("b"));
        assert_eq!(q.len(), 2);
        let latest = q.pop_latest().unwrap();
        assert_eq!(tag_of(&latest), "b");
        assert_eq!(q.len(), 0);
        assert!(q.is_empty());
        assert!(q.pop_latest().is_none());
    }

    #[test]
    fn zero_capacity_treated_as_one() {
        let mut q = ReportQueue::new(0);
        q.push(payload("a"));
        q.push(payload("b"));
        assert_eq!(q.len(), 1);
        assert_eq!(tag_of(&q.pop_latest().unwrap()), "b");
    }
}
