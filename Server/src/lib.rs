//! wtmd-server：WhatTheManDoingNEXT 服务器（阶段 2）。
//!
//! 职责：
//! - `config`：配置加载（缺失创建、损坏备份重建、校验）
//! - `state`：SQLite history + 内存当前状态（离线判定用 received_at）
//! - `routes`：envelope v1 API（上报 / 只读 / 公开只读 / 静态托管）
//! - `sse`：`/api/public/stream` 订阅广播（双帧 + KeepAlive 心跳注释）
//! - `ratelimit`：按客户端 IP 的滑动窗口限流
//! - `error`：envelope 错误与错误码

pub mod config;
pub mod error;
pub mod ratelimit;
pub mod routes;
pub mod sse;
pub mod state;

pub use config::Config;
pub use error::BuildError;
pub use state::AppState;

use std::sync::Arc;

use axum::Router;

/// 从 [`Config`] 构建完整应用（路由 + 状态 + 限流 + 静态托管）。
pub fn build_router(config: Config) -> Result<Router, BuildError> {
    let state = AppState::new(config)?;
    Ok(routes::router(state))
}

/// 用已构造的状态构建应用（测试可先拿到 state 再干预内存状态）。
pub fn build_router_with_state(state: Arc<AppState>) -> Result<Router, BuildError> {
    Ok(routes::router(state))
}
