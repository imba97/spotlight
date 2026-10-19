//! Windows 实现：分层遮罩窗口、屏幕截图、消息泵。
//!
//! 这一层的职责只有"把窗口摆上去"和"把屏幕读回来"，不含任何布局逻辑。

pub mod effect;
pub mod ipc;
pub mod message;
pub mod process;
pub mod shot;
pub mod text;

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

use spotlight_core::geom::Rect;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLACK_BRUSH, BitBlt, CAPTUREBLT, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetStockObject, ReleaseDC,
    SRCCOPY, ScreenToClient, SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWL_EXSTYLE, GetCursorPos,
    GetSystemMetrics, GetWindowLongPtrW, HTCLIENT, HTTRANSPARENT, LWA_ALPHA, MA_NOACTIVATE, MSG,
    PM_REMOVE, PeekMessageW, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN, SW_SHOWNOACTIVATE,
    SetLayeredWindowAttributes, SetWindowDisplayAffinity, ShowWindow, TranslateMessage,
    WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_NCHITTEST, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const CLASS_NAME: &str = "SpotlightOverlayWindow";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

static REGISTER: Once = Once::new();

/// 唯一可点击的区域（客户区坐标）。只有卡片上的关闭按钮会填进来。
static CLOSE_HIT: Mutex<Option<Rect>> = Mutex::new(None);
/// 用户点了关闭按钮。主线程在 tick 里消费。
static DISMISS_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn set_close_hit(rect: Option<Rect>) {
    if let Ok(mut guard) = CLOSE_HIT.lock() {
        *guard = rect;
    }
}

pub fn take_dismiss_request() -> bool {
    DISMISS_REQUESTED.swap(false, Ordering::SeqCst)
}

pub fn cursor_pos() -> (i32, i32) {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut pt);
    }
    (pt.x, pt.y)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // 除了关闭按钮，整块窗口对鼠标"不存在"。
            WM_NCHITTEST => {
                let hit = CLOSE_HIT.lock().ok().and_then(|guard| *guard);
                if let Some(rect) = hit {
                    let mut pt = POINT { x: 0, y: 0 };
                    GetCursorPos(&mut pt);
                    ScreenToClient(hwnd, &mut pt);
                    if rect.contains_point(pt.x, pt.y) {
                        return HTCLIENT as LRESULT;
                    }
                }
                HTTRANSPARENT as LRESULT
            }
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            WM_LBUTTONUP => {
                let hit = CLOSE_HIT.lock().ok().and_then(|guard| *guard);
                if let Some(rect) = hit {
                    let mut pt = POINT { x: 0, y: 0 };
                    GetCursorPos(&mut pt);
                    ScreenToClient(hwnd, &mut pt);
                    if rect.contains_point(pt.x, pt.y) {
                        DISMISS_REQUESTED.store(true, Ordering::SeqCst);
                        return 0;
                    }
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn ensure_class() -> Result<(), String> {
    let mut ok = true;
    REGISTER.call_once(|| unsafe {
        let name = wide(CLASS_NAME);
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: GetModuleHandleW(std::ptr::null()),
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: GetStockObject(BLACK_BRUSH),
            lpszMenuName: std::ptr::null(),
            lpszClassName: name.as_ptr(),
        };
        if RegisterClassW(&wc) == 0 {
            ok = false;
        }
    });
    if ok {
        Ok(())
    } else {
        Err("RegisterClassW failed".into())
    }
}

/// 遮罩窗口的扩展样式快照，供 `doctor` / 探针自检使用。
#[derive(Clone, Copy, Debug, Default)]
pub struct WindowStyles {
    pub layered: bool,
    pub transparent: bool,
    pub no_activate: bool,
    pub tool_window: bool,
    pub topmost: bool,
}

/// 一块纯色（黑）分层窗口，用统一 alpha 作为遮罩。
pub struct MaskWindow {
    hwnd: HWND,
}

impl MaskWindow {
    pub fn new(rect: Rect, alpha: u8) -> Result<Self, String> {
        ensure_class()?;
        if rect.is_empty() {
            return Err("empty rect".into());
        }
        let class = wide(CLASS_NAME);
        let title = wide("Spotlight");

        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return Err("CreateWindowExW failed".into());
            }
            // 先设 alpha 再显示，避免闪一下全黑。
            SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            Ok(Self { hwnd })
        }
    }

    /// 是否从屏幕捕获中排除。
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

    pub fn ex_style(&self) -> u32 {
        unsafe { GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32 }
    }

    /// 自检用：确认窗口确实是"分层、穿透、不抢焦点、不进 Alt+Tab、置顶"。
    pub fn styles(&self) -> WindowStyles {
        let ex = self.ex_style();
        WindowStyles {
            layered: ex & WS_EX_LAYERED != 0,
            transparent: ex & WS_EX_TRANSPARENT != 0,
            no_activate: ex & WS_EX_NOACTIVATE != 0,
            tool_window: ex & WS_EX_TOOLWINDOW != 0,
            topmost: ex & WS_EX_TOPMOST != 0,
        }
    }

    pub fn bits(&self) -> HWND {
        self.hwnd
    }
}

impl Drop for MaskWindow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}

/// 必须在创建任何窗口之前调用。
pub fn enable_dpi_awareness() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// 主屏 DPI 缩放，1.0 = 96 DPI。
pub fn primary_scale() -> f32 {
    unsafe { GetDpiForSystem() as f32 / 96.0 }
}

pub fn screen_size() -> (i32, i32) {
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

/// 处理消息直到 `d` 过去（用于让 DWM 把画面合成出来）。
pub fn pump_for(d: Duration) {
    let deadline = Instant::now() + d;
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        loop {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(8));
        }
    }
}

/// 一块屏幕区域的像素（BGRA，自上而下）。
pub struct Captured {
    pub width: i32,
    pub height: i32,
    pub bgra: Vec<u8>,
}

impl Captured {
    pub fn pixel(&self, x: i32, y: i32) -> [u8; 4] {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return [0, 0, 0, 0];
        }
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.bgra[i],
            self.bgra[i + 1],
            self.bgra[i + 2],
            self.bgra[i + 3],
        ]
    }
}

/// 用 BitBlt 抓一块屏幕区域。
pub fn capture_rect(x: i32, y: i32, w: i32, h: i32) -> Result<Captured, String> {
    if w <= 0 || h <= 0 {
        return Err("capture rect is empty".into());
    }
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return Err("GetDC failed".into());
        }
        let mem = CreateCompatibleDC(screen);
        if mem.is_null() {
            ReleaseDC(std::ptr::null_mut(), screen);
            return Err("CreateCompatibleDC failed".into());
        }

        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // 负值 = 自上而下
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
            return Err("CreateDIBSection failed".into());
        }

        let old = SelectObject(mem, bmp);
        let ok = BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT);
        let mut out = vec![0u8; (w * h * 4) as usize];
        if ok != 0 {
            std::ptr::copy_nonoverlapping(bits as *const u8, out.as_mut_ptr(), out.len());
        }

        SelectObject(mem, old);
        DeleteObject(bmp);
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), screen);

        if ok == 0 {
            return Err("BitBlt failed".into());
        }
        Ok(Captured {
            width: w,
            height: h,
            bgra: out,
        })
    }
}

/// 平均每像素亮度差（0–255），用于判断"画面有没有变化"。
pub fn mean_diff(a: &Captured, b: &Captured, region: Rect, areas: &[Rect]) -> f64 {
    let mut sum = 0.0f64;
    let mut count = 0u64;
    for area in areas {
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let (rx, ry) = (x - region.x, y - region.y);
                let pa = a.pixel(rx, ry);
                let pb = b.pixel(rx, ry);
                let d = (f64::from(pa[0]) - f64::from(pb[0])).abs()
                    + (f64::from(pa[1]) - f64::from(pb[1])).abs()
                    + (f64::from(pa[2]) - f64::from(pb[2])).abs();
                sum += d / 3.0;
                count += 1;
            }
        }
    }
    if count == 0 { 0.0 } else { sum / count as f64 }
}
