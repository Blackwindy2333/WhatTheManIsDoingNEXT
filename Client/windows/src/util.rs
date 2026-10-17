//! Win32 小工具：宽字符串转换、句柄判空、窗口文本读取。
//!
//! 句柄在不同 `windows-sys` 版本中可能是 `*mut c_void` 或整数别名，
//! 这里统一用泛型判空与 `usize` 往返转换，避免与具体表示绑定。

use std::ffi::c_void;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW;

/// `&str` → NUL 结尾 UTF-16。
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// NUL 结尾 UTF-16 缓冲 → `String`（在第一个 NUL 截断）。
pub fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// 句柄判空（兼容指针/整数两种句柄表示）。
pub fn is_null<T: PartialEq + Default>(h: T) -> bool {
    h == T::default()
}

/// HWND → usize（便于放进全局状态跨线程传递）。
#[allow(dead_code)]
pub fn hwnd_to_usize(h: HWND) -> usize {
    h as usize
}

/// usize → HWND。
#[allow(dead_code)]
pub fn usize_to_hwnd(v: usize) -> HWND {
    v as *mut c_void as HWND
}

/// 读取窗口/控件标题文本；空文本返回 `None`。
pub fn window_text(hwnd: HWND) -> Option<String> {
    unsafe {
        let len = GetWindowTextW(hwnd, std::ptr::null_mut(), 0);
        if len <= 0 {
            return None;
        }
        let mut buf = vec![0u16; (len as usize) + 1];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if n <= 0 {
            return None;
        }
        let text = from_wide(&buf[..n as usize]);
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }
}
