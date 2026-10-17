//! Config 加载/创建/损坏恢复测试 —— 只使用临时目录，绝不触碰 Server\config.json。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use wtmd_server::Config;

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    // 测试进程在受限沙箱下对系统 %TEMP% 无写权限；用 crate 内临时目录。
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join(".tmp");
    let dir = base.join(format!("{}-{n}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn missing_config_is_created_with_defaults() {
    let dir = temp_dir("missing");
    let path = dir.join("config.json");
    assert!(!path.exists());

    let config = Config::load_or_create(&path);
    assert!(path.exists(), "缺失时应创建默认文件");
    assert_eq!(config.port, 8080);
    assert_eq!(config.host, "127.0.0.1");
    assert_eq!(config.api_token, "change-me");
    assert_eq!(config.offline_after_seconds, 90);
    assert_eq!(config.history_limit, 500);
    assert_eq!(config.rate_limit_per_minute, 120);
    assert_eq!(config.show_history, true);
    assert_eq!(config.refresh_interval_seconds, 30);
    assert_eq!(config.trust_proxy, false);
    assert_eq!(config.web_dir, "Web");
    assert_eq!(config.cors_origins, vec!["*".to_string()]);
    assert!(!config.page_subtitle.is_empty(), "page_subtitle 必须有默认值");
    config.validate().expect("默认配置必须通过校验");

    // 再次加载读回自定义值。
    let mut custom = config.clone();
    custom.port = 9999;
    custom.save(&path).unwrap();
    let reloaded = Config::load_or_create(&path);
    assert_eq!(reloaded.port, 9999);
}

#[test]
fn corrupt_config_is_backed_up_and_rebuilt() {
    let dir = temp_dir("corrupt");
    let path = dir.join("config.json");
    std::fs::write(&path, "{ this is not json").unwrap();

    let config = Config::load_or_create(&path);
    let bak = dir.join("config.json.bak");
    assert!(bak.exists(), "损坏文件应改名为 config.json.bak");
    assert_eq!(config.port, 8080, "重建后应回到默认值");
    assert!(path.exists(), "应重建默认文件");
    let rebuilt: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(rebuilt["host"], "127.0.0.1");
}

#[test]
fn invalid_values_are_backed_up_and_rebuilt() {
    let dir = temp_dir("invalid");
    let path = dir.join("config.json");
    // port=0 越界（须在 1..=65535）。
    std::fs::write(&path, r#"{"port": 0, "offline_after_seconds": 90, "history_limit": 10}"#).unwrap();

    let config = Config::load_or_create(&path);
    assert!(dir.join("config.json.bak").exists(), "校验失败同样备份重建");
    assert_eq!(config.port, 8080);
    config.validate().unwrap();
}

#[test]
fn validate_rejects_out_of_range_values() {
    let mut config = Config::default();
    config.port = 70000;
    assert!(config.validate().is_err());
    let mut config = Config::default();
    config.offline_after_seconds = 5; // 须 >=10
    assert!(config.validate().is_err());
    let mut config = Config::default();
    config.history_limit = 0; // 须 >=1
    assert!(config.validate().is_err());
    let mut config = Config::default();
    config.host = String::new();
    assert!(config.validate().is_err());
}
