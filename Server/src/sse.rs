//! `/api/public/stream`：SSE 订阅广播。
//!
//! 每次上报广播两帧（契约确认见 lead m00069）：
//! - `event: update` 帧（供未来 `addEventListener("update")` 消费者）
//! - 默认 message 帧（供 `es.onmessage` 消费者，如 Web/ app.js）
//!
//! 两帧载荷均为 `{"type":"devices","data":[Device...]}`。心跳为 SSE 注释 `: ping`
//! （axum KeepAlive，15s），对 EventSource 透明。

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, Sse};
use serde_json::Value;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::state::AppState;

/// 广播消息：同一载荷的两种事件形态。
#[derive(Clone, Debug)]
pub enum SseFrame {
    /// `event: update` 帧。
    Update(Value),
    /// 默认 message 帧（`es.onmessage` 可见）。
    Message(Value),
}

const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// 构造 SSE 响应流：连接即发当前快照（双帧），之后跟随广播，空闲时发心跳注释。
pub fn sse_stream(
    state: Arc<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let snapshot = state.devices_message();
    let initial = tokio_stream::iter(vec![
        Ok(Event::default()
            .event("update")
            .data(snapshot.to_string())),
        Ok(Event::default().data(snapshot.to_string())),
    ]);

    let receiver = state.broadcast.subscribe();
    let live = BroadcastStream::new(receiver).filter_map(|item| match item {
        Ok(SseFrame::Update(payload)) => Some(Ok(Event::default()
            .event("update")
            .data(payload.to_string()))),
        Ok(SseFrame::Message(payload)) => Some(Ok(Event::default().data(payload.to_string()))),
        // 落后太多被跳过或通道关闭：忽略该条。
        Err(_) => None,
    });

    Sse::new(initial.chain(live)).keep_alive(
        KeepAlive::new()
            .interval(KEEP_ALIVE_INTERVAL)
            .text("ping"),
    )
}
