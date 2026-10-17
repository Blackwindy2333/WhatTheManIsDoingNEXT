//! 状态色托盘图标：内存 DC 绘制圆点（绿在线/黄暂停/灰停止），
//! `CreateIconIndirect` 生成 HICON；失败回退系统图标（tray.rs 处理）。

use std::ffi::c_void;
use std::mem::size_of;

use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, CreateSolidBrush, DeleteDC, DeleteObject,
    Ellipse, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HBRUSH,
    HDC, HGDIOBJ,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, LoadIconW, ICONINFO, IDI_APPLICATION,
};

use crate::util;

const SIZE: i32 = 32;

/// 状态色（COLORREF：0x00BBGGRR）。
pub fn color_green() -> u32 {
    0x00_22_C5_5E // 在线
}
pub fn color_yellow() -> u32 {
    0x00_2F_B6_F0 // 暂停（蓝黄偏暖）
}
pub fn color_gray() -> u32 {
    0x00_9C_9C_9C // 停止
}

/// 画一个圆点图标；失败返回 `None`。
pub fn make_status_icon(color: u32) -> Option<isize> {
    unsafe {
        let hdc: HDC = CreateCompatibleDC(std::ptr::null_mut());
        if util::is_null(hdc) {
            return None;
        }
        let mdc: HDC = CreateCompatibleDC(std::ptr::null_mut());
        if util::is_null(mdc) {
            DeleteDC(hdc);
            return None;
        }

        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = SIZE;
        bmi.bmiHeader.biHeight = -SIZE; // top-down
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;

        let mut bits: *mut c_void = std::ptr::null_mut();
        let color_bmp: HBITMAP = CreateDIBSection(
            hdc,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if util::is_null(color_bmp) {
            DeleteDC(mdc);
            DeleteDC(hdc);
            return None;
        }

        // 遮罩：1=透明。全 1 初始，再画不透明圆。
        let mask_bits = [0xFFu8; (SIZE as usize / 8) * SIZE as usize];
        let mask_bmp: HBITMAP = CreateBitmap(
            SIZE,
            SIZE,
            1,
            1,
            mask_bits.as_ptr() as *const c_void,
        );

        let old_color = SelectObject(hdc, color_bmp as HGDIOBJ);
        let old_mask = SelectObject(mdc, mask_bmp as HGDIOBJ);

        // 彩色圆点（背景保持透明）。
        let brush: HBRUSH = CreateSolidBrush(color);
        let old_brush = SelectObject(hdc, brush as HGDIOBJ);
        Ellipse(hdc, 2, 2, SIZE - 2, SIZE - 2);

        // 遮罩圆点（黑色=不透明）。
        let black: HBRUSH = CreateSolidBrush(0x0000_0000);
        SelectObject(mdc, black as HGDIOBJ);
        Ellipse(mdc, 2, 2, SIZE - 2, SIZE - 2);

        let mut info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask_bmp,
            hbmColor: color_bmp,
        };
        let icon = CreateIconIndirect(&mut info);

        // 清理 GDI 对象。
        SelectObject(hdc, old_brush);
        SelectObject(mdc, old_mask);
        SelectObject(hdc, old_color);
        DeleteObject(brush as HGDIOBJ);
        DeleteObject(black as HGDIOBJ);
        DeleteObject(mask_bmp as HGDIOBJ);
        DeleteObject(color_bmp as HGDIOBJ);
        DeleteDC(mdc);
        DeleteDC(hdc);

        if util::is_null(icon) {
            None
        } else {
            Some(icon as isize)
        }
    }
}

/// 系统默认应用图标（生成失败时的回退）。
pub fn fallback_icon() -> isize {
    unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) as isize }
}

/// 释放图标。
pub fn destroy_icon(icon: isize) {
    unsafe {
        if !util::is_null(icon) {
            DestroyIcon(icon as *mut c_void);
        }
    }
}
