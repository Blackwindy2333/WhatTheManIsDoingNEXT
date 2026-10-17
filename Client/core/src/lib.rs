//! wtmd-core：WhatTheManDoingNEXT 共享核心（Windows 客户端与 Android JNI 共用）。
//!
//! 职责（PLAN.md 阶段 1）：
//! - `config`：配置读写与默认创建（缺失创建、损坏备份重建）
//! - `protocol`：v1 上报协议与 envelope（错误码 0/40000/40100/40300/40400/42900/50000）
//! - `privacy`：隐私策略（窗口标题开关、黑名单脱敏、隐私暂停）
//! - `report`：HTTP 上报与重试（2xx/401/429/5xxx 分类、Retry-After 退避）
//! - `display_name`：进程/包名 → 显示名（内置映射 + 用户覆盖）
//! - `queue`：离线上报环形队列（满丢最旧、恢复后报最新）

pub mod config;
pub mod display_name;
pub mod privacy;
pub mod protocol;
pub mod queue;
pub mod report;

pub use config::{AppConfig, ConfigError};
pub use display_name::resolve_display_name;
pub use privacy::{build_report, is_blacklisted};
pub use protocol::{
    now_utc_iso, AppInfo, DeviceStatus, Envelope, ReportPayload, CODE_BAD_REQUEST, CODE_FORBIDDEN,
    CODE_INTERNAL, CODE_NOT_FOUND, CODE_OK, CODE_RATE_LIMITED, CODE_UNAUTHORIZED,
};
pub use queue::ReportQueue;
pub use report::{HttpPort, HttpResponse, ReportError, ReportOutcome, UreqHttp};
