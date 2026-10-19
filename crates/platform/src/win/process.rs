//! 单实例互斥与进程拉起。

use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::System::Threading::CreateMutexW;

/// 已经有一个实例在运行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlreadyRunning;

impl std::fmt::Display for AlreadyRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("已有实例在运行")
    }
}

impl std::error::Error for AlreadyRunning {}

/// 命名互斥量：保证同一会话里只有一个守护进程。
pub struct SingleInstance {
    handle: HANDLE,
}

impl SingleInstance {
    pub fn acquire(name: &str) -> Result<Self, AlreadyRunning> {
        let name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let handle = CreateMutexW(std::ptr::null(), 1, name.as_ptr());
            if handle.is_null() {
                return Err(AlreadyRunning);
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(handle);
                return Err(AlreadyRunning);
            }
            Ok(Self { handle })
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

/// 把本进程的标准句柄标记为不可继承。
///
/// `std::process::Command` 在 Windows 上总是带 `bInheritHandles=TRUE` 起进程，
/// 于是调用方的 stdout 管道会被守护进程继承——只要守护进程活着，管道就不关闭，
/// 调用方（比如 `spotlight ... | Out-Null`）会一直等下去。这就是之前偶发卡死的根因。
fn detach_std_handles() {
    use windows_sys::Win32::Foundation::{
        HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation,
    };
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };

    unsafe {
        for id in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let handle = GetStdHandle(id);
            if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

/// 拉起一个不占控制台的子进程（守护进程用）。
pub fn spawn_detached(exe: &Path, args: &[&str]) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    detach_std_handles();

    std::process::Command::new(exe)
        .args(args)
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}
