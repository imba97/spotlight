//! 线程消息总线。
//!
//! Win32 的窗口有线程亲和性：所有窗口操作必须发生在主线程。IPC 线程把任务丢进队列，
//! 再 `PostThreadMessage` 唤醒主线程，主线程从队列里取出来执行——这样上层不需要碰
//! 任何裸指针。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, RegisterHotKey, UnregisterHotKey, VK_ESCAPE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW,
    SetTimer, TranslateMessage, WM_APP, WM_HOTKEY, WM_TIMER,
};

/// 自定义唤醒消息。
const APP_COMMAND: u32 = WM_APP + 1;
/// 定时器 id，主线程只有一个定时器。
pub const TIMER_ID: usize = 1;
/// 热键 id。
pub const HOTKEY_ID: i32 = 1;

pub enum Event<T> {
    Command(T),
    Timer,
    /// 用户按下了"关掉画面"的热键。
    Hotkey,
}

/// 注册 `Ctrl+Alt+Esc`。被别的程序占用时返回 false。
pub fn register_dismiss_hotkey() -> bool {
    unsafe {
        RegisterHotKey(
            std::ptr::null_mut(),
            HOTKEY_ID,
            MOD_CONTROL | MOD_ALT,
            u32::from(VK_ESCAPE),
        ) != 0
    }
}

pub fn unregister_dismiss_hotkey() {
    unsafe {
        UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID);
    }
}

pub struct MessageBus<T> {
    queue: Arc<Mutex<VecDeque<T>>>,
    thread_id: u32,
}

impl<T: Send + 'static> MessageBus<T> {
    pub fn new() -> Self {
        // 先把消息队列建起来，否则 PostThreadMessage 会失败。
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
        }
        Self {
            queue: Arc::new(Mutex::new(VecDeque::new())),
            thread_id: unsafe { GetCurrentThreadId() },
        }
    }

    pub fn sender(&self) -> Sender<T> {
        Sender {
            queue: Arc::clone(&self.queue),
            thread_id: self.thread_id,
        }
    }

    /// 阻塞取下一条事件；返回 `None` 表示消息循环结束。
    pub fn next_event(&self) -> Option<Event<T>> {
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            loop {
                let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
                if ret <= 0 {
                    return None;
                }
                if msg.message == APP_COMMAND {
                    let item = self
                        .queue
                        .lock()
                        .ok()
                        .and_then(|mut queue| queue.pop_front());
                    if let Some(item) = item {
                        return Some(Event::Command(item));
                    }
                    continue;
                }
                if msg.message == WM_HOTKEY && msg.wParam == HOTKEY_ID as usize {
                    return Some(Event::Hotkey);
                }
                if msg.message == WM_TIMER {
                    return Some(Event::Timer);
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

impl<T: Send + 'static> Default for MessageBus<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Sender<T> {
    queue: Arc<Mutex<VecDeque<T>>>,
    thread_id: u32,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            queue: Arc::clone(&self.queue),
            thread_id: self.thread_id,
        }
    }
}

impl<T: Send> Sender<T> {
    pub fn send(&self, item: T) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.push_back(item);
        }
        unsafe {
            PostThreadMessageW(self.thread_id, APP_COMMAND, 0, 0);
        }
    }
}

/// 启动一个周期定时器；返回是否成功。
pub fn start_timer(ms: u32) -> bool {
    unsafe { SetTimer(std::ptr::null_mut(), TIMER_ID, ms, None) != 0 }
}

pub fn stop_timer() {
    unsafe {
        KillTimer(std::ptr::null_mut(), TIMER_ID);
    }
}
