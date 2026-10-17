//! wtmd-core：WhatTheManDoingNEXT 共享核心（Windows 客户端与 Android JNI 共用）。
//!
//! 职责（PLAN.md 阶段 1）：
//! - `config`：配置读写与默认创建（缺失创建、损坏备份重建）
//! - `protocol`：v1 上报协议与 envelope（错误码 0/40000/40100/40300/40400/42900/50000）
//! - `privacy`：隐私策略（窗口标题开关、黑名单脱敏、隐私暂停）
//! - `report`：HTTP 上报与重试（2xx/401/429/5xxx 分类、Retry-After 退避）
//! - `display_name`：进程/包名 → 显示名（内置映射 + 用户覆盖）
