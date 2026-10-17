//! 当前状态（内存）与历史（SQLite）。
//!
//! 离线判定一律用服务器接收时间 `received_at`：`online = (now - received_at) < offline_after_seconds`。

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::config::Config;
use crate::error::{ApiError, BuildError};
use crate::ratelimit::RateLimiter;
use crate::sse::SseFrame;

/// 合法上报状态（PLAN §8）。
pub const VALID_STATUSES: [&str; 4] = ["active", "paused", "idle", "stopped"];

/// UTC ISO-8601（Z 形式）。
pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

/// 上报体中的前台应用（null 表示无前台应用/隐私暂停）。
#[derive(Debug, Clone, Default)]
pub struct AppInfo {
    pub process_name: Option<String>,
    pub display_name: Option<String>,
    pub window_title: Option<String>,
}

impl AppInfo {
    pub fn to_json(&self) -> Value {
        json!({
            "process_name": self.process_name,
            "display_name": self.display_name,
            "window_title": self.window_title,
        })
    }
}

/// 校验后的上报输入。
#[derive(Debug, Clone)]
pub struct ReportInput {
    pub device_id: String,
    pub device_name: String,
    pub status: String,
    /// 客户端采集时间（ISO-8601 UTC，统一 Z 形式）。
    pub timestamp: String,
    pub app: Option<AppInfo>,
}

impl ReportInput {
    /// 严格校验上报体（非法返回 Err(message)，由路由层转 400/40000）。
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let device_id = match value.get("device_id") {
            Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
            Some(_) => return Err("device_id must be a non-empty string".to_string()),
            None => return Err("device_id is required".to_string()),
        };

        let device_name = match value.get("device_name") {
            None | Some(Value::Null) => device_id.clone(),
            Some(Value::String(s)) => {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    device_id.clone()
                } else {
                    trimmed.to_string()
                }
            }
            Some(_) => return Err("device_name must be a string".to_string()),
        };

        let status = match value.get("status").and_then(Value::as_str) {
            Some(s) if VALID_STATUSES.contains(&s) => s.to_string(),
            _ => {
                return Err(format!(
                    "status must be one of {}",
                    VALID_STATUSES.join(", ")
                ));
            }
        };

        let timestamp_raw = match value.get("timestamp").and_then(Value::as_str) {
            Some(s) if !s.trim().is_empty() => s.trim(),
            _ => return Err("timestamp must be an ISO-8601 UTC string".to_string()),
        };
        let parsed = chrono::DateTime::parse_from_rfc3339(timestamp_raw)
            .map_err(|_| format!("timestamp must be an ISO-8601 UTC string, got {timestamp_raw:?}"))?;
        let timestamp = parsed
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::AutoSi, true);

        let app = match value.get("app") {
            None | Some(Value::Null) => None,
            Some(Value::Object(map)) => {
                let mut info = AppInfo::default();
                for (key, slot) in [
                    ("process_name", &mut info.process_name),
                    ("display_name", &mut info.display_name),
                    ("window_title", &mut info.window_title),
                ] {
                    match map.get(key) {
                        None | Some(Value::Null) => {}
                        Some(Value::String(s)) => *slot = Some(s.clone()),
                        Some(_) => return Err(format!("app.{key} must be a string or null")),
                    }
                }
                Some(info)
            }
            Some(_) => return Err("app must be an object or null".to_string()),
        };

        Ok(ReportInput {
            device_id,
            device_name,
            status,
            timestamp,
            app,
        })
    }
}

/// 一台设备的当前状态（内存）。
#[derive(Debug, Clone)]
pub struct DeviceRecord {
    pub device_id: String,
    pub device_name: String,
    pub status: String,
    pub app: Option<AppInfo>,
    /// 客户端采集时间（ISO-8601 UTC）。
    pub timestamp: String,
    /// 服务器接收时间（离线判定依据）。
    pub received_at: DateTime<Utc>,
}

/// 一条历史记录（SQLite history 表一行）。
#[derive(Debug, Clone)]
pub struct HistoryRow {
    pub id: i64,
    pub device_id: String,
    pub device_name: Option<String>,
    pub status: Option<String>,
    pub process_name: Option<String>,
    pub display_name: Option<String>,
    pub window_title: Option<String>,
    pub timestamp: String,
}

impl HistoryRow {
    /// v1 形状（docs/API.md §3.3 + device_name）。
    pub fn to_v1_json(&self) -> Value {
        json!({
            "id": self.id,
            "device_id": self.device_id,
            "device_name": self.device_name,
            "status": self.status,
            "process_name": self.process_name,
            "display_name": self.display_name,
            "window_title": self.window_title,
            "timestamp": self.timestamp,
        })
    }

    /// 公开只读形状（Web/ 前端契约：display_name、process_name、status、timestamp）。
    pub fn to_public_json(&self) -> Value {
        json!({
            "display_name": self.display_name,
            "process_name": self.process_name,
            "status": self.status,
            "timestamp": self.timestamp,
        })
    }
}

/// 公开设备对象（Web/ 前端契约的 12 个字段，顺序无关）。
pub fn public_device_json(
    record: &DeviceRecord,
    now: DateTime<Utc>,
    offline_after_seconds: i64,
) -> Value {
    let age_seconds = (now - record.received_at).num_milliseconds() as f64 / 1000.0;
    let online = age_seconds < offline_after_seconds as f64;
    let healthy = online && record.status != "stopped" && record.status != "offline";
    json!({
        "id": record.device_id,
        "name": record.device_name,
        "enabled": true,
        "online": online,
        "status": record.status,
        "app": record.app.as_ref().map(AppInfo::to_json).unwrap_or(Value::Null),
        "timestamp": record.timestamp,
        "latency_ms": Value::Null,
        "http_status": Value::Null,
        "error": Value::Null,
        "updated_at": record.received_at.timestamp_millis() as f64 / 1000.0,
        "healthy": healthy,
    })
}

/// v1 设备对象（docs/API.md §4.2：status 结构 + online）。
pub fn v1_device_json(
    record: &DeviceRecord,
    now: DateTime<Utc>,
    offline_after_seconds: i64,
) -> Value {
    let age_seconds = (now - record.received_at).num_milliseconds() as f64 / 1000.0;
    let online = age_seconds < offline_after_seconds as f64;
    json!({
        "device_id": record.device_id,
        "device_name": record.device_name,
        "status": record.status,
        "app": record.app.as_ref().map(AppInfo::to_json).unwrap_or(Value::Null),
        "timestamp": record.timestamp,
        "updated_at": record.timestamp,
        "online": online,
    })
}

/// 应用状态：配置 + SQLite + 内存当前状态 + SSE 广播 + 限流器。
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Mutex<Connection>,
    pub devices: RwLock<HashMap<String, DeviceRecord>>,
    pub broadcast: broadcast::Sender<SseFrame>,
    pub limiter: RateLimiter,
    pub started_at: DateTime<Utc>,
}

impl AppState {
    /// 打开（创建）SQLite 并初始化表结构。
    pub fn new(config: Config) -> Result<Arc<Self>, BuildError> {
        if let Some(parent) = Path::new(&config.db_path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(&config.db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                device_id TEXT NOT NULL,
                device_name TEXT,
                status TEXT,
                process_name TEXT,
                display_name TEXT,
                window_title TEXT,
                timestamp TEXT NOT NULL,
                received_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_history_device_ts ON history(device_id, timestamp);",
        )?;
        let (broadcast, _) = broadcast::channel(64);
        let limiter = RateLimiter::new(config.rate_limit_per_minute);
        Ok(Arc::new(AppState {
            config: Arc::new(config),
            db: Mutex::new(conn),
            devices: RwLock::new(HashMap::new()),
            broadcast,
            limiter,
            started_at: Utc::now(),
        }))
    }

    /// 写入历史 + 更新内存当前状态（每设备保留最新 `history_limit` 条）。
    pub fn record_report(&self, input: &ReportInput) -> Result<DeviceRecord, ApiError> {
        let received_at = Utc::now();
        let received_str = received_at.to_rfc3339_opts(SecondsFormat::AutoSi, true);
        let (process_name, display_name, window_title) = match &input.app {
            Some(app) => (
                app.process_name.clone(),
                app.display_name.clone(),
                app.window_title.clone(),
            ),
            None => (None, None, None),
        };
        {
            let conn = self
                .db
                .lock()
                .map_err(|_| ApiError::internal("database lock poisoned"))?;
            conn.execute(
                "INSERT INTO history
                    (device_id, device_name, status, process_name, display_name, window_title, timestamp, received_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    input.device_id,
                    input.device_name,
                    input.status,
                    process_name,
                    display_name,
                    window_title,
                    input.timestamp,
                    received_str
                ],
            )
            .map_err(|e| ApiError::internal(format!("history insert failed: {e}")))?;
            conn.execute(
                "DELETE FROM history
                 WHERE device_id = ?1
                   AND id NOT IN (
                     SELECT id FROM history WHERE device_id = ?1 ORDER BY id DESC LIMIT ?2
                   )",
                params![input.device_id, self.config.history_limit],
            )
            .map_err(|e| ApiError::internal(format!("history trim failed: {e}")))?;
        }
        let record = DeviceRecord {
            device_id: input.device_id.clone(),
            device_name: input.device_name.clone(),
            status: input.status.clone(),
            app: input.app.clone(),
            timestamp: input.timestamp.clone(),
            received_at,
        };
        self.devices
            .write()
            .map_err(|_| ApiError::internal("state lock poisoned"))?
            .insert(input.device_id.clone(), record.clone());
        Ok(record)
    }

    /// 当前设备列表（按 device_name 排序）。
    pub fn devices_list(&self) -> Vec<DeviceRecord> {
        let map = self.devices.read().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<DeviceRecord> = map.values().cloned().collect();
        out.sort_by(|a, b| {
            a.device_name
                .to_lowercase()
                .cmp(&b.device_name.to_lowercase())
                .then_with(|| a.device_id.cmp(&b.device_id))
        });
        out
    }

    /// 设备是否存在（内存或历史中有记录）。
    pub fn device_exists(&self, device_id: &str) -> bool {
        if self
            .devices
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(device_id)
        {
            return true;
        }
        self.history(device_id, 1)
            .map(|rows| !rows.is_empty())
            .unwrap_or(false)
    }

    /// 历史（最新在前）。
    pub fn history(&self, device_id: &str, limit: i64) -> rusqlite::Result<Vec<HistoryRow>> {
        let conn = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, device_id, device_name, status, process_name, display_name, window_title, timestamp
             FROM history
             WHERE device_id = ?1
             ORDER BY id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![device_id, limit], |row| {
            Ok(HistoryRow {
                id: row.get(0)?,
                device_id: row.get(1)?,
                device_name: row.get(2)?,
                status: row.get(3)?,
                process_name: row.get(4)?,
                display_name: row.get(5)?,
                window_title: row.get(6)?,
                timestamp: row.get(7)?,
            })
        })?;
        rows.collect()
    }

    /// 公开设备数组（Web/ 前端契约形状）。
    pub fn public_devices_array(&self) -> Vec<Value> {
        let now = Utc::now();
        self.devices_list()
            .iter()
            .map(|record| {
                public_device_json(record, now, self.config.offline_after_seconds)
            })
            .collect()
    }

    /// v1 设备数组。
    pub fn v1_devices_array(&self) -> Vec<Value> {
        let now = Utc::now();
        self.devices_list()
            .iter()
            .map(|record| v1_device_json(record, now, self.config.offline_after_seconds))
            .collect()
    }

    /// SSE 广播载荷：`{"type":"devices","data":[Device...]}`。
    pub fn devices_message(&self) -> Value {
        json!({ "type": "devices", "data": self.public_devices_array() })
    }
}
