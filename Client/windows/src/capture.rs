//! Win32 前台窗口采集：
//! GetForegroundWindow → 窗口标题（GetWindowTextW）+ 窗口类名（GetClassNameW）
//! → GetWindowThreadProcessId → 进程名（QueryFullProcessImageNameW，
//! 失败回退 Toolhelp 快照，取 exe 文件名）。

use std::mem::size_of;

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId,
};

use crate::monitor::FgSample;
use crate::util;

/// 采样当前前台窗口；无前台窗口返回 `None`（→ Idle）。
pub fn sample_foreground() -> Option<FgSample> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if util::is_null(hwnd) {
            return None;
        }

        let mut class_buf = [0u16; 256];
        let class_len = GetClassNameW(hwnd, class_buf.as_mut_ptr(), class_buf.len() as i32);
        let class_name = if class_len > 0 {
            Some(util::from_wide(&class_buf[..class_len as usize]))
        } else {
            None
        };

        // 标题总是采集（变化检测需要），是否上报由 core 隐私规则决定。
        let window_title = util::window_text(hwnd);

        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let process_name = process_name(pid).unwrap_or_else(|| "Unknown".to_string());

        Some(FgSample {
            process_name,
            window_title,
            class_name,
        })
    }
}

/// pid → exe 文件名：QueryFullProcessImageNameW 优先，Toolhelp 快照回退。
fn process_name(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if !util::is_null(handle) {
            let mut buf = [0u16; 32768];
            let mut size = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size);
            CloseHandle(handle);
            if ok != 0 && size > 0 {
                let path = util::from_wide(&buf[..size as usize]);
                if let Some(name) = exe_file_name(&path) {
                    return Some(name);
                }
            }
        }
        toolhelp_name(pid)
    }
}

/// 取路径最后一段（exe 文件名）。
fn exe_file_name(path: &str) -> Option<String> {
    let name = path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Toolhelp 进程快照回退查找 exe 名。
unsafe fn toolhelp_name(pid: u32) -> Option<String> {
    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    if snapshot == INVALID_HANDLE_VALUE || util::is_null(snapshot) {
        return None;
    }
    let mut entry: PROCESSENTRY32W = std::mem::zeroed();
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut found = None;
    if Process32FirstW(snapshot, &mut entry) != 0 {
        loop {
            if entry.th32ProcessID == pid {
                found = Some(util::from_wide(&entry.szExeFile));
                break;
            }
            if Process32NextW(snapshot, &mut entry) == 0 {
                break;
            }
        }
    }
    CloseHandle(snapshot);
    found
}
