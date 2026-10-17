//! 进程/包名 → 显示名：用户覆盖映射优先，其次内置映射，未命中返回原名。
//! 匹配一律大小写不敏感。

use std::collections::BTreeMap;

/// 内置映射（键一律小写；匹配前先把输入转小写）。
const BUILTIN_MAP: &[(&str, &str)] = &[
    // Windows 常见进程
    ("code.exe", "Visual Studio Code"),
    ("chrome.exe", "Google Chrome"),
    ("firefox.exe", "Mozilla Firefox"),
    ("msedge.exe", "Microsoft Edge"),
    ("explorer.exe", "文件资源管理器"),
    ("cmd.exe", "命令提示符"),
    ("windowsterminal.exe", "Windows 终端"),
    ("powershell.exe", "PowerShell"),
    ("pwsh.exe", "PowerShell"),
    ("devenv.exe", "Visual Studio"),
    ("idea64.exe", "IntelliJ IDEA"),
    ("wechat.exe", "微信"),
    ("qq.exe", "QQ"),
    ("steam.exe", "Steam"),
    ("notepad.exe", "记事本"),
    ("notepad++.exe", "Notepad++"),
    ("calc.exe", "计算器"),
    ("taskmgr.exe", "任务管理器"),
    ("regedit.exe", "注册表编辑器"),
    ("control.exe", "控制面板"),
    ("systemsettings.exe", "设置"),
    ("pycharm64.exe", "PyCharm"),
    ("studio64.exe", "Android Studio"),
    ("obs64.exe", "OBS Studio"),
    ("spotify.exe", "Spotify"),
    ("discord.exe", "Discord"),
    ("telegram.exe", "Telegram"),
    ("zoom.exe", "Zoom"),
    ("dingtalk.exe", "钉钉"),
    ("feishu.exe", "飞书"),
    ("wps.exe", "WPS Office"),
    ("aliyunwangpan.exe", "阿里云盘"),
    // Android 常见包名
    ("com.tencent.mm", "微信"),
    ("com.tencent.mobileqq", "QQ"),
    ("com.alibaba.android.rimet", "钉钉"),
    ("com.ss.android.lark", "飞书"),
    ("org.telegram.messenger", "Telegram"),
    ("com.android.chrome", "Google Chrome"),
    ("com.microsoft.emmx", "Microsoft Edge"),
    ("com.zhiliaoapp.musically", "TikTok"),
    ("com.ss.android.ugc.aweme", "抖音"),
];

/// 解析显示名：`override_map` 精确键（大小写不敏感）优先 → 内置映射（大小写不敏感）→ 原名。
pub fn resolve_display_name(
    process_or_package: &str,
    override_map: &BTreeMap<String, String>,
) -> String {
    let lower = process_or_package.to_lowercase();
    for (key, value) in override_map {
        if key.to_lowercase() == lower {
            return value.clone();
        }
    }
    for (key, value) in BUILTIN_MAP {
        if *key == lower {
            return (*value).to_string();
        }
    }
    process_or_package.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_map_beats_builtin() {
        let mut map = BTreeMap::new();
        map.insert("code.exe".to_string(), "工作编辑器".to_string());
        assert_eq!(resolve_display_name("Code.exe", &map), "工作编辑器");
    }

    #[test]
    fn keys_match_case_insensitively() {
        let mut map = BTreeMap::new();
        map.insert("Chrome.EXE".to_string(), "我的浏览器".to_string());
        assert_eq!(resolve_display_name("chrome.exe", &map), "我的浏览器");
        assert_eq!(resolve_display_name("CHROME.EXE", &map), "我的浏览器");
    }

    #[test]
    fn builtin_match_case_insensitive() {
        let map = BTreeMap::new();
        assert_eq!(
            resolve_display_name("Code.EXE", &map),
            "Visual Studio Code"
        );
        assert_eq!(resolve_display_name("WINDOWSTERMINAL.EXE", &map), "Windows 终端");
        assert_eq!(resolve_display_name("com.tencent.mm", &map), "微信");
    }

    #[test]
    fn miss_returns_original() {
        let map = BTreeMap::new();
        assert_eq!(resolve_display_name("SomeWeird.exe", &map), "SomeWeird.exe");
        assert_eq!(resolve_display_name("", &map), "");
    }
}
