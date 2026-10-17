//! 开机自启：HKCU\Software\Microsoft\Windows\CurrentVersion\Run
//! 写/删值 `WhatTheManDoing` = exe 路径（REG_SZ）。

use std::mem::size_of;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY_CURRENT_USER,
    KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};

use crate::util;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// 注册表值名（任务书规定）。
pub const VALUE_NAME: &str = "WhatTheManDoing";

/// 注册命令：带引号的当前 exe 路径（路径含空格也安全）。
pub fn exe_command() -> String {
    let exe = std::env::current_exe().unwrap_or_else(|_| {
        crate::paths::exe_dir().join("wtmd-client-win.exe")
    });
    format!("\"{}\"", exe.display())
}

/// 当前是否已配置自启（值存在即视为开启）。
pub fn is_enabled() -> bool {
    unsafe {
        let mut key = std::mem::zeroed();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            util::wide(RUN_KEY).as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return false;
        }
        let mut size: u32 = 0;
        let rc = RegQueryValueExW(
            key,
            util::wide(VALUE_NAME).as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        );
        RegCloseKey(key);
        rc == ERROR_SUCCESS
    }
}

/// 写入自启值。
pub fn enable() -> Result<(), String> {
    let cmd = util::wide(&exe_command());
    unsafe {
        let mut key = std::mem::zeroed();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            util::wide(RUN_KEY).as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return Err("无法打开注册表 Run 键".to_string());
        }
        let rc = RegSetValueExW(
            key,
            util::wide(VALUE_NAME).as_ptr(),
            0,
            REG_SZ,
            cmd.as_ptr() as *const u8,
            (cmd.len() * size_of::<u16>()) as u32,
        );
        RegCloseKey(key);
        if rc != ERROR_SUCCESS {
            return Err(format!("写入 Run 值失败（错误码 {rc}）"));
        }
    }
    Ok(())
}

/// 删除自启值。
pub fn disable() -> Result<(), String> {
    unsafe {
        let mut key = std::mem::zeroed();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            util::wide(RUN_KEY).as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return Err("无法打开注册表 Run 键".to_string());
        }
        let rc = RegDeleteValueW(key, util::wide(VALUE_NAME).as_ptr());
        RegCloseKey(key);
        // 值本来就不存在视为成功。
        if rc != ERROR_SUCCESS && rc != ERROR_FILE_NOT_FOUND {
            return Err(format!("删除 Run 值失败（错误码 {rc}）"));
        }
    }
    Ok(())
}

/// 切换自启；返回切换后是否开启。
pub fn toggle() -> Result<bool, String> {
    if is_enabled() {
        disable()?;
        Ok(false)
    } else {
        enable()?;
        Ok(true)
    }
}
