//! 显示器枚举。
//!
//! 之前只拿 `GetSystemMetrics` 的主屏尺寸，只能在单屏下工作。这里改为真正的枚举：
//! 每块屏有自己的物理矩形、工作区、DPI 缩放，坐标可以是负数（副屏在左/在上）。

use spotlight_core::geom::Rect;
use spotlight_core::monitor::{Monitor, MonitorId};
use windows_sys::Win32::Foundation::{BOOL, LPARAM, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

unsafe extern "system" fn collect(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    unsafe {
        let list = &mut *(data as *mut Vec<HMONITOR>);
        list.push(monitor);
    }
    1
}

fn rect_of(rect: RECT) -> Rect {
    Rect::from_edges(rect.left, rect.top, rect.right, rect.bottom)
}

fn scale_of(monitor: HMONITOR) -> f32 {
    let (mut x, mut y) = (0u32, 0u32);
    let hr = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) };
    if hr == 0 && x > 0 {
        x as f32 / 96.0
    } else {
        1.0
    }
}

/// 按系统顺序返回所有显示器。`MonitorId` 就是这里的下标（0 起）。
pub fn enumerate() -> Vec<Monitor> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect),
            &mut handles as *mut Vec<HMONITOR> as LPARAM,
        );
    }

    let mut out = Vec::with_capacity(handles.len());
    for (index, monitor) in handles.into_iter().enumerate() {
        let mut info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        let ok = unsafe {
            GetMonitorInfoW(
                monitor,
                &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
            )
        };
        if ok == 0 {
            continue;
        }

        let name = String::from_utf16_lossy(&info.szDevice)
            .trim_end_matches('\0')
            .to_string();

        out.push(Monitor::new(
            MonitorId(index as u32),
            name,
            rect_of(info.monitorInfo.rcMonitor),
            rect_of(info.monitorInfo.rcWork),
            scale_of(monitor),
            info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        ));
    }
    out
}

/// 兜底：枚举失败时至少给出一块主屏，避免整个工具不可用。
pub fn enumerate_or_primary() -> Vec<Monitor> {
    let list = enumerate();
    if !list.is_empty() {
        return list;
    }
    let (w, h) = super::screen_size();
    vec![Monitor::new(
        MonitorId(0),
        "primary",
        Rect::new(0, 0, w, h),
        Rect::new(0, 0, w, h),
        super::primary_scale(),
        true,
    )]
}
