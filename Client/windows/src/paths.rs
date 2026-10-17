//! 路径约定：配置与日志放在 exe 同目录（PLAN.md §9 未指定具体路径时的缺省）。

use std::path::PathBuf;

/// exe 所在目录（取不到时退化为当前目录）。
pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 配置文件路径：`<exe 目录>\config.json`。
pub fn config_path() -> PathBuf {
    exe_dir().join("config.json")
}

/// 日志目录：`<exe 目录>\logs`。
pub fn log_dir() -> PathBuf {
    exe_dir().join("logs")
}

/// 日志文件路径：`<exe 目录>\logs\client-win.log`。
pub fn log_path() -> PathBuf {
    log_dir().join("client-win.log")
}
