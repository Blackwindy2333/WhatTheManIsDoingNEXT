//! 设置窗口：原生 Win32 窗口 + 控件（无 webview/egui，小内存）。
//! 「保存」→ validate() 通过后 save() 并热应用（写回 Shared 并唤醒采集线程）；
//! 「取消」/关闭按钮 → 关闭。视觉按 Apple Design 指南适度简化（见报告降级清单）。
//!
//! 死锁防护：窗口过程只瞬时持 Shared 锁；嵌套消息循环在不持锁时运行。

use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{GetStockObject, COLOR_WINDOW, DEFAULT_GUI_FONT};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetDlgItemTextW, GetMessageW, GetSystemMetrics, MessageBoxW, PostQuitMessage,
    RegisterClassW, SendMessageW, SetDlgItemTextW, SetForegroundWindow, ShowWindow,
    TranslateMessage, BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON,
    MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MSG, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW, WNDCLASSW,
    WM_APP, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_SETFONT, WS_BORDER, WS_CAPTION, WS_CHILD,
    WS_CLIPCHILDREN, WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT, WS_OVERLAPPED, WS_SYSMENU,
    WS_TABSTOP, WS_VISIBLE,
};

use wtmd_core::config::AppConfig;

use crate::monitor;
use crate::state::AppShared;
use crate::util;

// ── 控件 ID ─────────────────────────────────────────────────────────────
const ID_LBL_BASE: u32 = 2001;
const ID_ED_BASE: u32 = 2002;
const ID_LBL_DEVICE_ID: u32 = 2003;
const ID_ED_DEVICE_ID: u32 = 2004;
const ID_LBL_DEVICE_NAME: u32 = 2005;
const ID_ED_DEVICE_NAME: u32 = 2006;
const ID_LBL_TOKEN: u32 = 2007;
const ID_ED_TOKEN: u32 = 2008;
const ID_LBL_POLL: u32 = 2009;
const ID_ED_POLL: u32 = 2010;
const ID_CHK_TITLE: u32 = 2011;
const ID_LBL_BLACKLIST: u32 = 2012;
const ID_ED_BLACKLIST: u32 = 2013;
const ID_CHK_PAUSE: u32 = 2014;
const ID_BTN_SAVE: u32 = 2015;
const ID_BTN_CANCEL: u32 = 2016;
const ID_LBL_STATUS: u32 = 2017;

/// BST_CHECKED（避免引入 Win32_UI_Controls 特性）。
const BST_CHECKED: usize = 1;
/// EM_SETLIMITTEXT（同上，取 winuser.h 数值）。
const EM_SETLIMITTEXT: u32 = 0x00C5;

// ── 布局常量（像素）────────────────────────────────────────────────────
const PAD: i32 = 16;
const LABEL_W: i32 = 110;
const FIELD_H: i32 = 26;
const ROW_GAP: i32 = 12;
const WIN_W: i32 = 520;
const WIN_H: i32 = 520;

/// 设置窗口句柄（同时只开一个）。
static SETTINGS_HWND: AtomicIsize = AtomicIsize::new(0);
/// 嵌套消息循环退出标志（WM_DESTROY 置位）。
static LOOP_DONE: AtomicBool = AtomicBool::new(false);
/// 当前设置窗口关联的共享状态。
static SHARED: OnceLock<Mutex<Option<Arc<AppShared>>>> = OnceLock::new();

fn shared_cell() -> Option<Arc<AppShared>> {
    let cell = SHARED.get()?;
    let guard = cell.lock().unwrap_or_else(|e| e.into_inner());
    guard.clone()
}

/// 打开设置窗口：模态语义（内部消息循环，直到窗口关闭或收到 WM_QUIT）。
pub fn open(shared: Arc<AppShared>) {
    unsafe {
        // 已经开着就置顶即可。
        let existing = SETTINGS_HWND.load(Ordering::SeqCst);
        if existing != 0 {
            let hwnd = existing as HWND;
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
            return;
        }
        let _ = SHARED.set(Mutex::new(None));
        if let Some(cell) = SHARED.get() {
            *cell.lock().unwrap_or_else(|e| e.into_inner()) = Some(shared);
        }
        LOOP_DONE.store(false, Ordering::SeqCst);

        let class_name = util::wide("WtmdSettingsWindow");
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: ptr::null_mut(),
            hIcon: ptr::null_mut(),
            hCursor: ptr::null_mut(),
            hbrBackground: (COLOR_WINDOW + 1) as *mut _,
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        RegisterClassW(&wc); // 重复注册返回 0，可忽略

        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
        let ex = WS_EX_CONTROLPARENT;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: WIN_W,
            bottom: WIN_H,
        };
        AdjustWindowRectEx(&mut rect, style, 0, ex);
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        let x = (GetSystemMetrics(SM_CXSCREEN) - w) / 2;
        let y = (GetSystemMetrics(SM_CYSCREEN) - h) / 2;

        let hwnd = CreateWindowExW(
            ex,
            class_name.as_ptr(),
            util::wide("WhatTheManDoing 设置").as_ptr(),
            style,
            x,
            y,
            w,
            h,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        );
        if util::is_null(hwnd) {
            if let Some(cell) = SHARED.get() {
                *cell.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
            return;
        }
        SETTINGS_HWND.store(hwnd as isize, Ordering::SeqCst);

        if let Some(cfg) = shared_cell().map(|s| s.lock().cfg.clone()) {
            build_controls(hwnd, &cfg);
        }
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);

        // 模态消息循环：消息照常派发（托盘保持响应）；
        // 窗口销毁后退出；若收到 WM_QUIT（如托盘"退出"），补投并透传给外层。
        let mut msg: MSG = std::mem::zeroed();
        loop {
            let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if ret <= 0 {
                // WM_QUIT 或错误：退出嵌套循环并把 WM_QUIT 交回主循环。
                PostQuitMessage(0);
                break;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
            if LOOP_DONE.load(Ordering::SeqCst) {
                break;
            }
        }

        SETTINGS_HWND.store(0, Ordering::SeqCst);
        if let Some(cell) = SHARED.get() {
            *cell.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }
}

/// 控件创建辅助（自动挂系统 GUI 字体）。
unsafe fn make_child(
    hwnd: HWND,
    class: &str,
    text: &str,
    style: u32,
    ex: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: u32,
) -> HWND {
    let c = util::wide(class);
    let t = util::wide(text);
    let child = CreateWindowExW(
        ex,
        c.as_ptr(),
        t.as_ptr(),
        style | WS_CHILD | WS_VISIBLE,
        x,
        y,
        w,
        h,
        hwnd,
        id as *mut _,
        ptr::null_mut(),
        ptr::null(),
    );
    let font = GetStockObject(DEFAULT_GUI_FONT);
    SendMessageW(child, WM_SETFONT, font as WPARAM, 1);
    child
}

unsafe fn check_box(hwnd: HWND, id: u32, checked: bool) {
    let child = windows_sys::Win32::UI::WindowsAndMessaging::GetDlgItem(hwnd, id as i32);
    SendMessageW(child, BM_SETCHECK, if checked { BST_CHECKED } else { 0 }, 0);
}

unsafe fn build_controls(hwnd: HWND, cfg: &AppConfig) {
    let mut y = PAD;
    let field_w = WIN_W - 2 * PAD - LABEL_W;

    make_child(hwnd, "STATIC", "服务器地址", 0, 0, PAD, y + 4, LABEL_W, FIELD_H, ID_LBL_BASE);
    make_child(
        hwnd, "EDIT", &cfg.api_base_url,
        WS_TABSTOP | WS_BORDER, WS_EX_CLIENTEDGE,
        PAD + LABEL_W, y, field_w, FIELD_H, ID_ED_BASE,
    );
    y += FIELD_H + ROW_GAP;

    make_child(hwnd, "STATIC", "设备 ID", 0, 0, PAD, y + 4, LABEL_W, FIELD_H, ID_LBL_DEVICE_ID);
    make_child(
        hwnd, "EDIT", &cfg.device_id,
        WS_TABSTOP | WS_BORDER, WS_EX_CLIENTEDGE,
        PAD + LABEL_W, y, field_w, FIELD_H, ID_ED_DEVICE_ID,
    );
    y += FIELD_H + ROW_GAP;

    make_child(hwnd, "STATIC", "设备名称", 0, 0, PAD, y + 4, LABEL_W, FIELD_H, ID_LBL_DEVICE_NAME);
    make_child(
        hwnd, "EDIT", &cfg.device_name,
        WS_TABSTOP | WS_BORDER, WS_EX_CLIENTEDGE,
        PAD + LABEL_W, y, field_w, FIELD_H, ID_ED_DEVICE_NAME,
    );
    y += FIELD_H + ROW_GAP;

    make_child(hwnd, "STATIC", "API Token", 0, 0, PAD, y + 4, LABEL_W, FIELD_H, ID_LBL_TOKEN);
    make_child(
        hwnd, "EDIT", &cfg.api_token,
        WS_TABSTOP | WS_BORDER, WS_EX_CLIENTEDGE,
        PAD + LABEL_W, y, field_w, FIELD_H, ID_ED_TOKEN,
    );
    y += FIELD_H + ROW_GAP;

    make_child(hwnd, "STATIC", "采样间隔 (ms)", 0, 0, PAD, y + 4, LABEL_W, FIELD_H, ID_LBL_POLL);
    let poll = make_child(
        hwnd, "EDIT", &cfg.poll_interval_ms.to_string(),
        WS_TABSTOP | WS_BORDER, WS_EX_CLIENTEDGE,
        PAD + LABEL_W, y, field_w, FIELD_H, ID_ED_POLL,
    );
    SendMessageW(poll, EM_SETLIMITTEXT, 9, 0);
    y += FIELD_H + ROW_GAP;

    make_child(
        hwnd, "BUTTON", "上报窗口标题（隐私敏感）",
        WS_TABSTOP | BS_AUTOCHECKBOX as u32, 0,
        PAD, y, WIN_W - 2 * PAD, FIELD_H, ID_CHK_TITLE,
    );
    check_box(hwnd, ID_CHK_TITLE, cfg.report_window_title);
    y += FIELD_H + ROW_GAP;

    make_child(hwnd, "STATIC", "应用黑名单", 0, 0, PAD, y + 4, LABEL_W, FIELD_H, ID_LBL_BLACKLIST);
    make_child(
        hwnd, "EDIT", &monitor::format_blacklist(&cfg.app_name_blacklist),
        WS_TABSTOP | WS_BORDER, WS_EX_CLIENTEDGE,
        PAD + LABEL_W, y, field_w, FIELD_H, ID_ED_BLACKLIST,
    );
    y += FIELD_H + ROW_GAP;

    make_child(
        hwnd, "BUTTON", "隐私暂停采集",
        WS_TABSTOP | BS_AUTOCHECKBOX as u32, 0,
        PAD, y, WIN_W - 2 * PAD, FIELD_H, ID_CHK_PAUSE,
    );
    check_box(hwnd, ID_CHK_PAUSE, cfg.privacy_pause);
    y += FIELD_H + ROW_GAP + 8;

    // 状态行（内联校验反馈，Apple Design "反馈即刻"）。
    make_child(hwnd, "STATIC", "", 0, 0, PAD, y, WIN_W - 2 * PAD, 40, ID_LBL_STATUS);
    y += 48;

    make_child(
        hwnd, "BUTTON", "保存",
        WS_TABSTOP | BS_DEFPUSHBUTTON as u32, 0,
        WIN_W - PAD - 220, y, 100, 32, ID_BTN_SAVE,
    );
    make_child(
        hwnd, "BUTTON", "取消",
        WS_TABSTOP, 0,
        WIN_W - PAD - 100, y, 100, 32, ID_BTN_CANCEL,
    );
}

unsafe fn set_status(hwnd: HWND, text: &str) {
    SetDlgItemTextW(hwnd, ID_LBL_STATUS as i32, util::wide(text).as_ptr());
}

unsafe fn get_text(hwnd: HWND, id: u32) -> String {
    let mut buf = [0u16; 1024];
    GetDlgItemTextW(hwnd, id as i32, buf.as_mut_ptr(), buf.len() as i32);
    util::from_wide(&buf)
}

unsafe fn is_checked(hwnd: HWND, id: u32) -> bool {
    let child = windows_sys::Win32::UI::WindowsAndMessaging::GetDlgItem(hwnd, id as i32);
    SendMessageW(child, BM_GETCHECK, 0, 0) as usize == BST_CHECKED
}

/// 「保存」：组装 AppConfig → validate() → save() → 热应用 → 关闭。
unsafe fn on_save(hwnd: HWND) {
    let shared = match shared_cell() {
        Some(s) => s,
        None => return,
    };
    let base = get_text(hwnd, ID_ED_BASE).trim().to_string();
    let device_id = get_text(hwnd, ID_ED_DEVICE_ID).trim().to_string();
    let device_name = get_text(hwnd, ID_ED_DEVICE_NAME).trim().to_string();
    let api_token = get_text(hwnd, ID_ED_TOKEN);
    let poll_text = get_text(hwnd, ID_ED_POLL).trim().to_string();
    let report_title = is_checked(hwnd, ID_CHK_TITLE);
    let blacklist_text = get_text(hwnd, ID_ED_BLACKLIST);
    let privacy_pause = is_checked(hwnd, ID_CHK_PAUSE);

    let poll_interval_ms: u64 = match poll_text.parse() {
        Ok(v) => v,
        Err(_) => {
            set_status(hwnd, "⚠ 采样间隔必须是正整数（毫秒）");
            return;
        }
    };
    let blacklist = monitor::parse_blacklist(&blacklist_text);

    // 在当前配置基础上覆盖基本项（history_limit、display_name_map 保留）。
    let mut cfg = shared.lock().cfg.clone();
    cfg.api_base_url = base;
    cfg.device_id = device_id;
    cfg.device_name = device_name;
    cfg.api_token = api_token;
    cfg.poll_interval_ms = poll_interval_ms;
    cfg.report_window_title = report_title;
    cfg.app_name_blacklist = blacklist;
    cfg.privacy_pause = privacy_pause;

    if let Err(e) = cfg.validate() {
        set_status(hwnd, &format!("⚠ 配置校验失败：{e}"));
        return;
    }
    let config_path = shared.lock().config_path.clone();
    if let Err(e) = cfg.save(&config_path) {
        set_status(hwnd, &format!("⚠ 保存失败：{e}"));
        return;
    }

    {
        let mut st = shared.lock();
        st.cfg = cfg;
        st.report_note = "配置已更新".to_string();
    }
    shared.notify();
    shared.balloon("设置已保存", "新配置已即时生效");
    crate::tray::request_refresh();
    DestroyWindow(hwnd);
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    _lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = (wparam as u32) & 0xFFFF;
            match id {
                ID_BTN_SAVE => {
                    on_save(hwnd);
                    0
                }
                ID_BTN_CANCEL => {
                    DestroyWindow(hwnd);
                    0
                }
                _ => DefWindowProcW(hwnd, msg, wparam, _lparam),
            }
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            LOOP_DONE.store(true, Ordering::SeqCst);
            // 唤醒可能阻塞的 GetMessage（空消息即可）。
            let _ = windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                GetCurrentThreadId(),
                WM_APP + 3,
                0,
                0,
            );
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, _lparam),
    }
}

/// 校验失败时的模态提示（错误反馈）。
#[allow(dead_code)]
unsafe fn alert(hwnd: HWND, text: &str) {
    MessageBoxW(
        hwnd,
        util::wide(text).as_ptr(),
        util::wide("WhatTheManDoing").as_ptr(),
        MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
    );
}
