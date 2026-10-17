//! 系统托盘：Shell_NotifyIconW 图标 + 右键菜单 + 气泡 + tooltip。
//! 菜单：显示当前状态 / 暂停(恢复)采集 / 停止(开始)采集 / 设置… /
//! 开机自启（勾选）/ 退出。
//!
//! 死锁防护：`TrackPopupMenu` 与 `settings::open` 都跑嵌套消息循环，
//! 期间托盘窗口消息可能回调 wnd_proc，因此所有 TRAY 锁只做瞬时快照/写入，
//! 不允许在持锁状态下调用消息循环或 UI 函数。

use std::mem::size_of;
use std::ptr;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    LoadIconW, PostMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow,
    TrackPopupMenu, CS_HREDRAW, CS_VREDRAW, HICON, IDI_APPLICATION, MF_CHECKED, MF_SEPARATOR,
    MF_STRING, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WNDCLASSW, WM_APP, WM_COMMAND,
    WM_LBUTTONUP, WM_RBUTTONUP,
};

use crate::icon;
use crate::logging;
use crate::state::AppShared;
use crate::util;
use wtmd_core::protocol::DeviceStatus;

/// 托盘图标回调消息。
const WM_TRAY: u32 = WM_APP + 1;
/// 状态刷新消息（worker/UI 线程发来）。
const WM_TRAY_REFRESH: u32 = WM_APP + 2;

// 菜单命令 ID。
const IDM_SHOW_STATUS: usize = 1001;
const IDM_TOGGLE_PAUSE: usize = 1002;
const IDM_TOGGLE_RUN: usize = 1003;
const IDM_SETTINGS: usize = 1004;
const IDM_AUTOSTART: usize = 1005;
const IDM_EXIT: usize = 1006;

const TRAY_UID: u32 = 1;

/// 托盘内部状态（单例）。hwnd 存 isize：原始指针不满足 `Send`。
struct TrayInner {
    shared: Arc<AppShared>,
    hwnd: isize,
    icon: isize,
}

static TRAY: OnceLock<Mutex<Option<TrayInner>>> = OnceLock::new();
static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);

/// 瞬时锁内取值：禁止在 f 内跑消息循环。
fn with_tray<R>(f: impl FnOnce(&mut TrayInner) -> R) -> Option<R> {
    let cell = TRAY.get()?;
    let mut guard = cell.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_mut().map(f)
}

/// 初始化托盘：注册窗口类、创建消息窗口、添加图标。
pub fn init(shared: Arc<AppShared>) -> Result<HWND, String> {
    unsafe {
        let class_name = util::wide("WtmdTrayWindow");
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: std::ptr::null_mut(),
            hIcon: LoadIconW(ptr::null_mut(), IDI_APPLICATION),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        if RegisterClassW(&wc) == 0 {
            return Err("注册托盘窗口类失败".to_string());
        }
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            util::wide("WhatTheManDoing").as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            ptr::null(),
        );
        if util::is_null(hwnd) {
            return Err("创建托盘消息窗口失败".to_string());
        }
        TRAY_HWND.store(hwnd as isize, Ordering::SeqCst);

        let mut icon_handle =
            icon::make_status_icon(icon::color_green()).unwrap_or_else(icon::fallback_icon);
        if !add_icon(hwnd, icon_handle) {
            icon::destroy_icon(icon_handle);
            icon_handle = icon::fallback_icon();
            let _ = add_icon(hwnd, icon_handle);
        }

        let _ = TRAY.set(Mutex::new(Some(TrayInner {
            shared,
            hwnd: hwnd as isize,
            icon: icon_handle,
        })));
        with_tray(|t| update_tooltip(t));
        Ok(hwnd)
    }
}

/// 清理托盘图标（退出时调用）。
pub fn cleanup() {
    if let Some(cell) = TRAY.get() {
        let taken = {
            let mut guard = cell.lock().unwrap_or_else(|e| e.into_inner());
            guard.take()
        };
        if let Some(t) = taken {
            unsafe {
                let mut data = empty_data(t.hwnd as HWND);
                Shell_NotifyIconW(NIM_DELETE, &mut data);
                DestroyWindow(t.hwnd as HWND);
            }
            icon::destroy_icon(t.icon);
        }
    }
}

/// 请求刷新（tooltip/图标/气泡），可从任意线程调用。
pub fn request_refresh() {
    let hwnd = TRAY_HWND.load(Ordering::SeqCst);
    if hwnd != 0 {
        unsafe {
            PostMessageW(hwnd as HWND, WM_TRAY_REFRESH, 0, 0);
        }
    }
}

unsafe fn empty_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = std::mem::zeroed();
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = TRAY_UID;
    data
}

unsafe fn add_icon(hwnd: HWND, icon_handle: isize) -> bool {
    let mut data = empty_data(hwnd);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = icon_handle as HICON;
    Shell_NotifyIconW(NIM_ADD, &mut data) != 0
}

unsafe fn update_tooltip(t: &mut TrayInner) {
    let text = {
        let st = t.shared.lock();
        format!(
            "{} · {}",
            st.cfg.device_name,
            crate::monitor::status_text(st.status)
        )
    };
    let wide = util::wide(&text);
    let mut data = empty_data(t.hwnd as HWND);
    data.uFlags = NIF_TIP;
    for (i, w) in wide.iter().take(127).enumerate() {
        data.szTip[i] = *w;
    }
    Shell_NotifyIconW(NIM_MODIFY, &mut data);
}

unsafe fn show_balloon(t: &TrayInner, title: &str, body: &str) {
    let tw = util::wide(title);
    let bw = util::wide(body);
    let mut data = empty_data(t.hwnd as HWND);
    data.uFlags = NIF_INFO;
    data.dwInfoFlags = NIIF_INFO;
    for (i, w) in tw.iter().take(63).enumerate() {
        data.szInfoTitle[i] = *w;
    }
    for (i, w) in bw.iter().take(255).enumerate() {
        data.szInfo[i] = *w;
    }
    Shell_NotifyIconW(NIM_MODIFY, &mut data);
}

unsafe fn replace_icon(t: &mut TrayInner, status: DeviceStatus) {
    let color = match status {
        DeviceStatus::Active => icon::color_green(),
        DeviceStatus::Paused => icon::color_yellow(),
        DeviceStatus::Idle | DeviceStatus::Stopped => icon::color_gray(),
    };
    let new_icon = icon::make_status_icon(color).unwrap_or_else(icon::fallback_icon);
    let mut data = empty_data(t.hwnd as HWND);
    data.uFlags = NIF_ICON;
    data.hIcon = new_icon as HICON;
    if Shell_NotifyIconW(NIM_MODIFY, &mut data) != 0 {
        let old = t.icon;
        t.icon = new_icon;
        icon::destroy_icon(old);
    } else {
        icon::destroy_icon(new_icon);
    }
}

/// 弹出右键菜单并处理选择（调用时不持 TRAY 锁）。
unsafe fn popup_and_handle(shared: &Arc<AppShared>, hwnd: HWND) {
    // 快照菜单文案后立即放锁（TrackPopupMenu 是消息循环）。
    let (pause_label, run_label, autostart_checked) = {
        let st = shared.lock();
        (
            if st.cfg.privacy_pause {
                "恢复采集".to_string()
            } else {
                "暂停采集".to_string()
            },
            if st.capture_running {
                "停止采集".to_string()
            } else {
                "开始采集".to_string()
            },
            crate::autostart::is_enabled(),
        )
    };

    let menu = CreatePopupMenu();
    if util::is_null(menu) {
        return;
    }
    AppendMenuW(
        menu,
        MF_STRING,
        IDM_SHOW_STATUS,
        util::wide("显示当前状态").as_ptr(),
    );
    AppendMenuW(menu, MF_STRING, IDM_TOGGLE_PAUSE, util::wide(&pause_label).as_ptr());
    AppendMenuW(menu, MF_STRING, IDM_TOGGLE_RUN, util::wide(&run_label).as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
    AppendMenuW(menu, MF_STRING, IDM_SETTINGS, util::wide("设置…").as_ptr());
    let autostart_flags = if autostart_checked {
        MF_STRING | MF_CHECKED
    } else {
        MF_STRING
    };
    AppendMenuW(
        menu,
        autostart_flags,
        IDM_AUTOSTART,
        util::wide("开机自启").as_ptr(),
    );
    AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
    AppendMenuW(menu, MF_STRING, IDM_EXIT, util::wide("退出").as_ptr());

    SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(
        menu,
        TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
        0,
        0,
        0,
        hwnd,
        ptr::null(),
    );
    DestroyMenu(menu);
    if cmd != 0 {
        handle_menu_command(shared, cmd as usize);
    }
}

/// 菜单命令处理（调用时不持 TRAY 锁；气泡经 pending_balloon 展示）。
fn handle_menu_command(shared: &Arc<AppShared>, cmd: usize) {
    match cmd {
        IDM_SHOW_STATUS => {
            let body = {
                let st = shared.lock();
                format!(
                    "设备：{}\n状态：{}\n当前应用：{}\n上报：{}",
                    st.cfg.device_name,
                    crate::monitor::status_text(st.status),
                    st.current_app.as_deref().unwrap_or("（无）"),
                    st.report_note
                )
            };
            shared.balloon("当前状态", &body);
        }
        IDM_TOGGLE_PAUSE => {
            let (paused, saved) = {
                let mut st = shared.lock();
                st.cfg.privacy_pause = !st.cfg.privacy_pause;
                let saved = st.cfg.save(&st.config_path);
                (st.cfg.privacy_pause, saved)
            };
            match saved {
                Ok(()) => {
                    logging::info(&format!("隐私暂停 -> {paused}"));
                    shared.balloon(
                        "设置已更新",
                        if paused {
                            "已暂停采集（隐私模式）"
                        } else {
                            "已恢复采集"
                        },
                    );
                }
                Err(e) => {
                    logging::error(&format!("保存配置失败：{e}"));
                    shared.balloon("保存失败", &format!("{e}"));
                }
            }
            shared.notify();
        }
        IDM_TOGGLE_RUN => {
            let running = {
                let mut st = shared.lock();
                st.capture_running = !st.capture_running;
                st.capture_running
            };
            logging::info(&format!("采集开关 -> {running}"));
            shared.balloon(
                "采集状态",
                if running { "已开始采集" } else { "已停止采集" },
            );
            shared.notify();
        }
        IDM_SETTINGS => {
            // 嵌套消息循环：此刻不持任何锁。
            crate::settings::open(shared.clone());
        }
        IDM_AUTOSTART => match crate::autostart::toggle() {
            Ok(on) => {
                logging::info(&format!("开机自启 -> {on}"));
                shared.balloon(
                    "开机自启",
                    if on { "已开启开机自启" } else { "已关闭开机自启" },
                );
            }
            Err(e) => {
                logging::error(&format!("开机自启设置失败：{e}"));
                shared.balloon("设置失败", &e);
            }
        },
        IDM_EXIT => {
            {
                let mut st = shared.lock();
                st.shutdown = true;
                st.capture_running = false;
            }
            logging::info("用户请求退出");
            shared.notify();
            unsafe {
                PostQuitMessage(0);
            }
        }
        _ => {}
    }
    request_refresh();
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAY => {
            let mouse = (lparam as u32) & 0xFFFF;
            if mouse == WM_RBUTTONUP || mouse == WM_LBUTTONUP {
                // 取出 shared/hwnd 后立刻放锁：popup 与命令处理都是消息循环。
                let snapshot = with_tray(|t| (t.shared.clone(), t.hwnd));
                if let Some((shared, hwnd)) = snapshot {
                    if mouse == WM_RBUTTONUP {
                        popup_and_handle(&shared, hwnd as HWND);
                    } else {
                        // 左键单击：显示当前状态。
                        handle_menu_command(&shared, IDM_SHOW_STATUS);
                    }
                }
            }
            0
        }
        WM_TRAY_REFRESH => {
            with_tray(|t| {
                let (status, balloon) = {
                    let mut st = t.shared.lock();
                    (st.status, st.pending_balloon.take())
                };
                replace_icon(t, status);
                update_tooltip(t);
                if let Some((title, body)) = balloon {
                    show_balloon(t, &title, &body);
                }
            });
            0
        }
        WM_COMMAND => {
            let cmd = (wparam as u32 & 0xFFFF) as usize;
            let snapshot = with_tray(|t| t.shared.clone());
            if let Some(shared) = snapshot {
                handle_menu_command(&shared, cmd);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
