//! 用系统字体把文字画成一张 8 位 alpha 掩膜。
//!
//! 走 GDI 而不是自带字形排版器，理由只有一条：**不增体积、不扫字体目录**，冷启动更快。
//! 代价是排版能力有限（只做换行和左对齐），后续要富文本时再换 DirectWrite。

use std::ffi::c_void;

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLACK_BRUSH, CLIP_DEFAULT_PRECIS,
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
    DIB_RGB_COLORS, DT_CALCRECT, DT_NOPREFIX, DT_WORDBREAK, DeleteDC, DeleteObject, DrawTextW,
    FF_DONTCARE, FW_NORMAL, FillRect, GetDC, GetStockObject, OUT_TT_PRECIS, ReleaseDC,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};

const MAX_HEIGHT: i32 = 512;

pub struct TextBitmap {
    pub width: u32,
    pub height: u32,
    /// 每像素 8 位覆盖率，行优先。
    pub alpha: Vec<u8>,
    /// 文字超过高度上限、被裁掉了一部分。调用方应当告诉用户，而不是默默截断。
    pub clipped: bool,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 把 `text` 排版进 `max_width`，返回裁剪后的 alpha 掩膜。
pub fn render_text(text: &str, max_width: u32, font_px: i32) -> Option<TextBitmap> {
    if text.trim().is_empty() {
        return None;
    }
    let max_width = max_width.clamp(32, 1024) as i32;
    let font_px = font_px.clamp(8, 96);
    let utf16: Vec<u16> = text.encode_utf16().collect();

    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return None;
        }
        let mem = CreateCompatibleDC(screen);
        if mem.is_null() {
            ReleaseDC(std::ptr::null_mut(), screen);
            return None;
        }

        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: max_width,
            biHeight: -MAX_HEIGHT,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(mem, &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if bmp.is_null() || bits.is_null() {
            DeleteDC(mem);
            ReleaseDC(std::ptr::null_mut(), screen);
            return None;
        }

        let old_bmp = SelectObject(mem, bmp);

        // 黑底、白字：稍后把亮度当作覆盖率。
        let full = RECT {
            left: 0,
            top: 0,
            right: max_width,
            bottom: MAX_HEIGHT,
        };
        FillRect(mem, &full, GetStockObject(BLACK_BRUSH));

        let face = wide("Microsoft YaHei UI");
        let font = CreateFontW(
            -font_px,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_TT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            ANTIALIASED_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            face.as_ptr(),
        );
        let old_font = SelectObject(mem, font);
        SetBkMode(mem, TRANSPARENT as i32);
        SetTextColor(mem, 0x00FF_FFFF);

        // 先量高度
        let mut measured = RECT {
            left: 0,
            top: 0,
            right: max_width,
            bottom: 0,
        };
        DrawTextW(
            mem,
            utf16.as_ptr(),
            utf16.len() as i32,
            &mut measured,
            DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
        );
        let width = measured.right.clamp(1, max_width);
        let height = measured.bottom.clamp(1, MAX_HEIGHT);

        let mut draw_rect = RECT {
            left: 0,
            top: 0,
            right: max_width,
            bottom: MAX_HEIGHT,
        };
        DrawTextW(
            mem,
            utf16.as_ptr(),
            utf16.len() as i32,
            &mut draw_rect,
            DT_WORDBREAK | DT_NOPREFIX,
        );

        let mut alpha = vec![0u8; (width * height) as usize];
        let pixels =
            std::slice::from_raw_parts(bits as *const u8, (max_width * MAX_HEIGHT * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let idx = ((y * max_width + x) * 4) as usize;
                // BGRA：白字在黑底上，三个通道相同；取最大值即可。
                let v = pixels[idx].max(pixels[idx + 1]).max(pixels[idx + 2]);
                alpha[(y * width + x) as usize] = v;
            }
        }

        SelectObject(mem, old_font);
        DeleteObject(font);
        SelectObject(mem, old_bmp);
        DeleteObject(bmp);
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), screen);

        let clipped = measured.bottom >= MAX_HEIGHT;
        Some(TextBitmap {
            width: width as u32,
            height: height as u32,
            alpha,
            clipped,
        })
    }
}
