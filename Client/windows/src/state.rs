//! 线程间共享状态：采集线程与托盘/设置 UI 通过 Mutex + Condvar 协作。

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use wtmd_core::config::AppConfig;
use wtmd_core::protocol::DeviceStatus;

/// UI 与采集线程共享的全部状态。
pub struct Shared {
    /// 当前生效配置（设置保存/托盘开关/文件热加载都会更新）。
    pub cfg: AppConfig,
    /// 配置文件路径。
    pub config_path: PathBuf,
    /// 当前逻辑状态（Active/Paused/Idle/Stopped，用于托盘展示）。
    pub status: DeviceStatus,
    /// 当前前台应用显示文本（如 `chrome.exe`），无则 `None`。
    pub current_app: Option<String>,
    /// 上报状态文案（如 `已上报`、`离线缓存 3 条`、`token 无效`）。
    pub report_note: String,
    /// false = 托盘"停止采集"。
    pub capture_running: bool,
    /// true = 进程退出中。
    pub shutdown: bool,
    /// 待展示的托盘气泡（标题、正文），UI 线程取走后置 `None`。
    pub pending_balloon: Option<(String, String)>,
}

/// 共享句柄。
pub struct AppShared {
    pub inner: Mutex<Shared>,
    pub cv: Condvar,
}

impl AppShared {
    pub fn new(cfg: AppConfig, config_path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Shared {
                status: DeviceStatus::Active,
                current_app: None,
                report_note: "尚未上报".to_string(),
                cfg,
                config_path,
                capture_running: true,
                shutdown: false,
                pending_balloon: None,
            }),
            cv: Condvar::new(),
        })
    }

    pub fn lock(&self) -> MutexGuard<'_, Shared> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 唤醒采集线程（命令/配置变更后调用）。
    pub fn notify(&self) {
        self.cv.notify_all();
    }

    /// 采集线程等待命令；超时返回继续轮询。
    pub fn wait_timeout<'a>(
        &self,
        guard: MutexGuard<'a, Shared>,
        dur: Duration,
    ) -> MutexGuard<'a, Shared> {
        self.cv
            .wait_timeout(guard, dur)
            .unwrap_or_else(|e| e.into_inner())
            .0
    }

    /// 设置气泡提示（下一 UI 刷新时展示）。
    pub fn balloon(&self, title: &str, body: &str) {
        let mut st = self.lock();
        st.pending_balloon = Some((title.to_string(), body.to_string()));
    }
}
