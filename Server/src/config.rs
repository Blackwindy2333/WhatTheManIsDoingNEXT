//! 服务器配置：默认值、加载、校验、缺失创建、损坏备份重建。

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 配置文件名（当前目录优先，其次 `Server/`）。
pub const CONFIG_FILE_NAME: &str = "config.json";

fn default_host() -> String {
    "127.0.0.1".to_string()
}
fn default_port() -> i64 {
    8080
}
fn default_db_path() -> String {
    "wtmd.db".to_string()
}
fn default_api_token() -> String {
    "change-me".to_string()
}
fn default_offline_after_seconds() -> i64 {
    90
}
fn default_history_limit() -> i64 {
    500
}
fn default_cors_origins() -> Vec<String> {
    vec!["*".to_string()]
}
fn default_rate_limit_per_minute() -> i64 {
    120
}
fn default_page_title() -> String {
    "What The Man Doing".to_string()
}
fn default_page_subtitle() -> String {
    "前台应用监控聚合看板".to_string()
}
fn default_show_history() -> bool {
    true
}
fn default_refresh_interval_seconds() -> i64 {
    30
}
fn default_trust_proxy() -> bool {
    false
}
fn default_web_dir() -> String {
    "Web".to_string()
}

/// 服务器配置。缺失字段按默认值补齐（`#[serde(default)]`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub host: String,
    pub port: i64,
    pub db_path: String,
    pub api_token: String,
    pub offline_after_seconds: i64,
    pub history_limit: i64,
    pub cors_origins: Vec<String>,
    pub rate_limit_per_minute: i64,
    pub page_title: String,
    pub page_subtitle: String,
    pub show_history: bool,
    pub refresh_interval_seconds: i64,
    pub trust_proxy: bool,
    pub web_dir: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            host: default_host(),
            port: default_port(),
            db_path: default_db_path(),
            api_token: default_api_token(),
            offline_after_seconds: default_offline_after_seconds(),
            history_limit: default_history_limit(),
            cors_origins: default_cors_origins(),
            rate_limit_per_minute: default_rate_limit_per_minute(),
            page_title: default_page_title(),
            page_subtitle: default_page_subtitle(),
            show_history: default_show_history(),
            refresh_interval_seconds: default_refresh_interval_seconds(),
            trust_proxy: default_trust_proxy(),
            web_dir: default_web_dir(),
        }
    }
}

impl Config {
    /// 校验关键字段（PLAN §8：port 1..=65535、offline_after_seconds>=10、history_limit>=1）。
    pub fn validate(&self) -> Result<(), String> {
        if self.port < 1 || self.port > 65535 {
            return Err(format!("port must be between 1 and 65535, got {}", self.port));
        }
        if self.offline_after_seconds < 10 {
            return Err(format!(
                "offline_after_seconds must be >= 10, got {}",
                self.offline_after_seconds
            ));
        }
        if self.history_limit < 1 {
            return Err(format!("history_limit must be >= 1, got {}", self.history_limit));
        }
        if self.host.trim().is_empty() {
            return Err("host must be a non-empty string".to_string());
        }
        if self.db_path.trim().is_empty() {
            return Err("db_path must be a non-empty string".to_string());
        }
        if self.web_dir.trim().is_empty() {
            return Err("web_dir must be a non-empty string".to_string());
        }
        Ok(())
    }

    /// 保存完整配置（pretty JSON）。
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        text.push('\n');
        fs::write(path, text)
    }

    /// 加载配置；文件缺失则创建默认，解析失败或校验失败则备份为 `config.json.bak` 并重建默认。
    /// 永不失败：任何问题都回落到默认值。
    pub fn load_or_create(path: &Path) -> Config {
        if path.exists() {
            match fs::read_to_string(path) {
                Ok(text) => match serde_json::from_str::<Config>(&text) {
                    Ok(cfg) => match cfg.validate() {
                        Ok(()) => return cfg,
                        Err(msg) => eprintln!(
                            "[wtmd-server] config invalid at {}: {}; backing up and rebuilding defaults",
                            path.display(),
                            msg
                        ),
                    },
                    Err(msg) => eprintln!(
                        "[wtmd-server] config corrupt at {}: {}; backing up and rebuilding defaults",
                        path.display(),
                        msg
                    ),
                },
                Err(msg) => eprintln!(
                    "[wtmd-server] cannot read {}: {}; rebuilding defaults",
                    path.display(),
                    msg
                ),
            }
            backup_config(path);
        } else {
            eprintln!(
                "[wtmd-server] config missing at {}; creating defaults",
                path.display()
            );
        }
        let cfg = Config::default();
        if let Err(msg) = cfg.save(path) {
            eprintln!("[wtmd-server] failed to write {}: {}", path.display(), msg);
        }
        cfg
    }

    /// 工作区加载顺序：当前目录 `config.json`，否则 `Server/config.json`；都缺失则在当前目录创建。
    pub fn load_workspace_config() -> (Config, PathBuf) {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let local = cwd.join(CONFIG_FILE_NAME);
        if local.exists() {
            return (Config::load_or_create(&local), local);
        }
        let nested = cwd.join("Server").join(CONFIG_FILE_NAME);
        if nested.exists() {
            return (Config::load_or_create(&nested), nested);
        }
        (Config::load_or_create(&local), local)
    }
}

fn backup_config(path: &Path) {
    let bak = path.with_extension("json.bak");
    // Windows 上 rename 不覆盖已存在目标，先清掉旧备份。
    if bak.exists() {
        let _ = fs::remove_file(&bak);
    }
    if let Err(msg) = fs::rename(path, &bak) {
        eprintln!(
            "[wtmd-server] failed to back up {} to {}: {}",
            path.display(),
            bak.display(),
            msg
        );
    } else {
        eprintln!(
            "[wtmd-server] backed up {} to {}",
            path.display(),
            bak.display()
        );
    }
}
