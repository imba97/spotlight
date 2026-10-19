//! 逐像素 alpha 的效果窗口（光环、箭头、卡片都画在这里）。

use std::ffi::c_void;

use spotlight_core::geom::Rect;
use windows_sys::Win32::Foundation::{HWND, POINT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GWL_EXSTYLE, GetWindowLongPtrW, SW_SHOWNOACTIVATE,
    SetWindowDisplayAffinity, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WDA_EXCLUDEFROMCAPTURE,
    WDA_NONE, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
    WS_POPUP,
};

use super::{WindowStyles, ensure_class, wide};

/// 一块用 `UpdateLayeredWindow` 提交位图的置顶窗口。
pub struct EffectWindow {
    hwnd: HWND,
}

impl EffectWindow {
    pub fn new(rect: Rect) -> Result<Self, String> {
        ensure_class()?;
        if rect.is_empty() {
            return Err("empty effect rect".into());
        }
        let class = wide(super::CLASS_NAME);
        let title = wide("Spotlight");

        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return Err("CreateWindowExW(effect) failed".into());
            }
            Ok(Self { hwnd })
        }
    }

    /// 提交一张预乘 BGRA 位图，并把它摆到屏幕坐标 `(x, y)`。
    pub fn set_pixels(&self, x: i32, y: i32, w: i32, h: i32, bgra: &[u8]) -> bool {
        if w <= 0 || h <= 0 || bgra.len() < (w as usize) * (h as usize) * 4 {
            return false;
        }
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            if screen.is_null() {
                return false;
            }
            let mem = CreateCompatibleDC(screen);
            if mem.is_null() {
                ReleaseDC(std::ptr::null_mut(), screen);
                return false;
            }

            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bmp =
                CreateDIBSection(mem, &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if bmp.is_null() || bits.is_null() {
                DeleteDC(mem);
                ReleaseDC(std::ptr::null_mut(), screen);
                return false;
            }

            std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u8, bgra.len());
            let old = SelectObject(mem, bmp);

            let dst = POINT { x, y };
            let size = SIZE { cx: w, cy: h };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let ok = UpdateLayeredWindow(
                self.hwnd, screen, &dst, &size, mem, &src, 0, &blend, ULW_ALPHA,
            );

            SelectObject(mem, old);
            DeleteObject(bmp);
            DeleteDC(mem);
            ReleaseDC(std::ptr::null_mut(), screen);

            if ok != 0 {
                ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                true
            } else {
                false
            }
        }
    }

    pub fn set_excluded(&self, excluded: bool) -> bool {
        unsafe {
            SetWindowDisplayAffinity(
                self.hwnd,
                if excluded {
                    WDA_EXCLUDEFROMCAPTURE
                } else {
                    WDA_NONE
                },
            ) != 0
        }
    }

    pub fn styles(&self) -> WindowStyles {
        let ex = unsafe { GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32 };
        WindowStyles {
            layered: ex & WS_EX_LAYERED != 0,
            transparent: ex & WS_EX_TRANSPARENT != 0,
            no_activate: ex & WS_EX_NOACTIVATE != 0,
            tool_window: ex & WS_EX_TOOLWINDOW != 0,
            topmost: ex & WS_EX_TOPMOST != 0,
        }
    }
}

impl Drop for EffectWindow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}
