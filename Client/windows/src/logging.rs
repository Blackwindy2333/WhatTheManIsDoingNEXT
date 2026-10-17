//! 简单文件日志：`logs/client-win.log`，超过 1MB 轮转为 `.log.1`；
//! 同时保留控制台输出。错误与状态切换由调用方记入。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use wtmd_core::protocol::now_utc_iso;

/// 轮转阈值：1MB。
const MAX_LOG_BYTES: u64 = 1_048_576;

static LOG: OnceLock<Mutex<(File, PathBuf)>> = OnceLock::new();

/// 初始化日志：建目录、超限轮转、打开追加句柄。
pub fn init(log_path: &Path) {
    if let Some(dir) = log_path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    rotate_if_needed(log_path);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .expect("无法打开日志文件");
    let _ = LOG.set(Mutex::new((file, log_path.to_path_buf())));
    info(&format!("日志已启动：{}", log_path.display()));
}

fn rotate_if_needed(log_path: &Path) {
    if let Ok(meta) = fs::metadata(log_path) {
        if meta.len() > MAX_LOG_BYTES {
            let backup = log_path.with_extension("log.1");
            let _ = fs::remove_file(&backup);
            let _ = fs::rename(log_path, backup);
        }
    }
}

fn write_line(level: &str, msg: &str) {
    let line = format!("[{}] [{level}] {msg}", now_utc_iso());
    // 控制台输出保留（release 包无控制台时静默忽略错误）。
    if level == "ERROR" {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
    if let Some(log) = LOG.get() {
        if let Ok(mut guard) = log.lock() {
            let (file, path) = &mut *guard;
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
            // 简化滚动：写超阈值即轮转到 .log.1。
            if let Ok(meta) = fs::metadata(path.as_path()) {
                if meta.len() > MAX_LOG_BYTES {
                    let _ = file.flush();
                    let backup = path.with_extension("log.1");
                    let _ = fs::remove_file(&backup);
                    let _ = fs::rename(path.as_path(), &backup);
                    if let Ok(new_file) = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path.as_path())
                    {
                        *file = new_file;
                    }
                }
            }
        }
    }
}

pub fn info(msg: &str) {
    write_line("INFO", msg);
}

pub fn error(msg: &str) {
    write_line("ERROR", msg);
}
