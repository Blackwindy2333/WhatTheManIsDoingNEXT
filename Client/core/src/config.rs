//! 配置读写与校验（PLAN.md 阶段 1 / §9：缺失以默认创建、损坏备份重建）。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// 客户端配置。字段沿用 `agent/config.json`（PLAN.md 阶段 1）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppConfig {
    /// 服务器地址，默认 `http://127.0.0.1:8080`。
    pub api_base_url: String,
    /// 设备 ID，默认 `my-device`。
    pub device_id: String,
    /// 设备显示名，默认 `My Device`。
    pub device_name: String,
    /// Bearer token，默认 `change-me`。
    pub api_token: String,
    /// 采样间隔（毫秒），默认 1000，须 >= 100。
    pub poll_interval_ms: u64,
    /// 是否上报窗口标题，默认 false（默认不采集不上报）。
    pub report_window_title: bool,
    /// 黑名单（大小写不敏感子串匹配），默认空。
    pub app_name_blacklist: Vec<String>,
    /// 隐私暂停：开启后一律 `status=paused` 且 `app=None`。
    pub privacy_pause: bool,
    /// 历史条数上限（本地/服务端约定），默认 500。
    pub history_limit: u64,
    /// 用户显示名覆盖映射（精确键、大小写不敏感）。
    pub display_name_map: BTreeMap<String, String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            api_base_url: "http://127.0.0.1:8080".to_string(),
            device_id: "my-device".to_string(),
            device_name: "My Device".to_string(),
            api_token: "change-me".to_string(),
            poll_interval_ms: 1000,
            report_window_title: false,
            app_name_blacklist: Vec::new(),
            privacy_pause: false,
            history_limit: 500,
            display_name_map: BTreeMap::new(),
        }
    }
}

/// 配置错误：IO / JSON 解析 / 语义校验。
#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(serde_json::Error),
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "config io error: {e}"),
            ConfigError::Parse(e) => write!(f, "config parse error: {e}"),
            ConfigError::Invalid(msg) => write!(f, "config invalid: {msg}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        ConfigError::Io(e)
    }
}

impl From<serde_json::Error> for ConfigError {
    fn from(e: serde_json::Error) -> Self {
        ConfigError::Parse(e)
    }
}

/// 损坏文件的备份路径：`<原名>.bak`（如 `config.json` → `config.json.bak`）。
fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".bak");
    PathBuf::from(name)
}

impl AppConfig {
    /// 读取配置：文件缺失→写入默认并返回默认；JSON 损坏→备份为 `<原名>.bak` 后重建默认。
    ///
    /// JSON 可解析但语义非法（见 [`AppConfig::validate`]）时返回
    /// [`ConfigError::Invalid`] 且不改动原文件，交由调用方处置。
    pub fn load_or_create(path: &Path) -> Result<AppConfig, ConfigError> {
        if !path.exists() {
            let cfg = AppConfig::default();
            cfg.save(path)?;
            return Ok(cfg);
        }
        let text = std::fs::read_to_string(path)?;
        match serde_json::from_str::<AppConfig>(&text) {
            Ok(cfg) => {
                cfg.validate()?;
                Ok(cfg)
            }
            Err(_) => {
                let backup = backup_path(path);
                if backup.exists() {
                    std::fs::remove_file(&backup)?;
                }
                std::fs::rename(path, &backup)?;
                let cfg = AppConfig::default();
                cfg.save(path)?;
                Ok(cfg)
            }
        }
    }

    /// 保存配置为格式化 JSON（自动创建父目录）。
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// 校验：`api_base_url` 须以 `http://` 或 `https://` 开头；`poll_interval_ms >= 100`；`device_id` 非空。
    pub fn validate(&self) -> Result<(), ConfigError> {
        let url = self.api_base_url.trim();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(ConfigError::Invalid(
                "api_base_url must start with http:// or https://".to_string(),
            ));
        }
        if self.poll_interval_ms < 100 {
            return Err(ConfigError::Invalid(
                "poll_interval_ms must be >= 100".to_string(),
            ));
        }
        if self.device_id.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "device_id must not be empty".to_string(),
            ));
        }
        Ok(())
    }

    /// 拼出上报地址 `<base>/api/v1/report`（忽略首尾空白与尾斜杠）。
    pub fn report_url(&self) -> String {
        format!(
            "{}/api/v1/report",
            self.api_base_url.trim().trim_end_matches('/')
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// 测试用临时目录句柄：测试结束（含 panic 展开）自动删除，不留下产物。
    struct TempDir(PathBuf);

    impl std::ops::Deref for TempDir {
        type Target = PathBuf;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 每个测试一个独立临时目录。
    ///
    /// 首选 `std::env::temp_dir()`（系统临时目录）。注意：受管环境（如本机 DSH
    /// 沙箱）会禁止"在工作区内构建出的测试进程"写系统临时目录及任何工作区外
    /// 位置（一律返回 PermissionDenied），此时退回 crate 内 `target/test-tmp/`
    /// ——`target/` 是构建产物树，已被根 .gitignore 的 `target/` 规则忽略。
    /// 无论使用哪个位置，测试结束都会自动清理。
    fn temp_dir() -> TempDir {
        let name = format!(
            "config-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        let candidates = [
            std::env::temp_dir().join(&name),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("test-tmp")
                .join(&name),
        ];
        for dir in candidates {
            let _ = std::fs::remove_dir_all(&dir);
            if std::fs::create_dir_all(&dir).is_ok() {
                return TempDir(dir);
            }
        }
        panic!("cannot create test temp dir under system temp or crate target/");
    }

    #[test]
    fn missing_file_creates_default_and_loads() {
        let dir = temp_dir();
        let path = dir.join("config.json");
        let cfg = AppConfig::load_or_create(&path).unwrap();
        assert_eq!(cfg, AppConfig::default());
        cfg.validate().unwrap();
        assert!(path.exists(), "default config file must be written");
        // 再次加载一致，且内容可解析
        let text = std::fs::read_to_string(&path).unwrap();
        let parsed: AppConfig = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, AppConfig::default());
        let again = AppConfig::load_or_create(&path).unwrap();
        assert_eq!(again, cfg);
    }

    #[test]
    fn corrupt_json_is_backed_up_and_default_rebuilt() {
        let dir = temp_dir();
        let path = dir.join("config.json");
        let garbage = "{ this is not json";
        std::fs::write(&path, garbage).unwrap();
        let cfg = AppConfig::load_or_create(&path).unwrap();
        assert_eq!(cfg, AppConfig::default());
        let backup = dir.join("config.json.bak");
        assert!(backup.exists(), "corrupt file must be backed up to .bak");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), garbage);
        // 新文件为合法默认配置
        let text = std::fs::read_to_string(&path).unwrap();
        let parsed: AppConfig = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, AppConfig::default());
    }

    #[test]
    fn validate_rejects_bad_url() {
        let mut cfg = AppConfig::default();
        cfg.api_base_url = "ftp://127.0.0.1".to_string();
        assert!(matches!(cfg.validate(), Err(ConfigError::Invalid(_))));
        cfg.api_base_url = "127.0.0.1:8080".to_string();
        assert!(matches!(cfg.validate(), Err(ConfigError::Invalid(_))));
        cfg.api_base_url = String::new();
        assert!(matches!(cfg.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn validate_rejects_short_poll_interval() {
        let mut cfg = AppConfig::default();
        cfg.poll_interval_ms = 99;
        assert!(matches!(cfg.validate(), Err(ConfigError::Invalid(_))));
        cfg.poll_interval_ms = 100;
        cfg.validate().unwrap();
    }

    #[test]
    fn validate_rejects_empty_device_id() {
        let mut cfg = AppConfig::default();
        cfg.device_id = String::new();
        assert!(matches!(cfg.validate(), Err(ConfigError::Invalid(_))));
        cfg.device_id = "   ".to_string();
        assert!(matches!(cfg.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn save_load_round_trip() {
        let dir = temp_dir();
        let path = dir.join("nested").join("config.json");
        let mut cfg = AppConfig::default();
        cfg.api_base_url = "https://example.com/".to_string();
        cfg.device_id = "pc-01".to_string();
        cfg.device_name = "书房电脑".to_string();
        cfg.api_token = "secret-token".to_string();
        cfg.poll_interval_ms = 2500;
        cfg.report_window_title = true;
        cfg.app_name_blacklist = vec!["secret".to_string(), "game".to_string()];
        cfg.privacy_pause = true;
        cfg.history_limit = 123;
        cfg.display_name_map
            .insert("Code.exe".to_string(), "工作编辑器".to_string());
        cfg.save(&path).unwrap();
        let loaded = AppConfig::load_or_create(&path).unwrap();
        assert_eq!(loaded, cfg);
    }

    #[test]
    fn report_url_handles_trailing_slash() {
        let mut cfg = AppConfig::default();
        cfg.api_base_url = "http://127.0.0.1:8080".to_string();
        let no_slash = cfg.report_url();
        assert_eq!(no_slash, "http://127.0.0.1:8080/api/v1/report");
        cfg.api_base_url = "http://127.0.0.1:8080/".to_string();
        assert_eq!(cfg.report_url(), no_slash);
        cfg.api_base_url = "http://127.0.0.1:8080///".to_string();
        assert_eq!(cfg.report_url(), no_slash);
    }
}
