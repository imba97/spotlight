//! 命名管道 IPC：一行一个 JSON 消息。
//!
//! 服务端在同一进程里一次只处理一个连接——调用方都是短命的 CLI，串行足够。

use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_GENERIC_READ, FILE_GENERIC_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

/// windows-sys 没有导出这个常量：`PIPE_ACCESS_DUPLEX`。
const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;

const MAX_MESSAGE: usize = 1 << 20; // 1 MiB

pub fn pipe_name() -> String {
    // TODO(security): 按 05 章改为带 UserSID 的管道名 + SDDL 限制。
    r"\\.\pipe\spotlight-default".to_string()
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 一个已连接的管道会话。
pub struct Connection {
    file: File,
    buf: Vec<u8>,
}

impl Connection {
    /// 读一行；连接关闭返回 `Ok(None)`。
    pub fn recv(&mut self) -> Result<Option<String>, String> {
        loop {
            if let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=pos).collect();
                let text = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
                return Ok(Some(text));
            }
            let mut chunk = [0u8; 4096];
            let n = self.file.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                return Ok(None);
            }
            self.buf.extend_from_slice(&chunk[..n]);
            if self.buf.len() > MAX_MESSAGE {
                return Err("message too large".into());
            }
        }
    }

    pub fn send(&mut self, line: &str) -> Result<(), String> {
        self.file
            .write_all(line.as_bytes())
            .and_then(|_| self.file.write_all(b"\n"))
            .and_then(|_| self.file.flush())
            .map_err(|e| e.to_string())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        unsafe {
            DisconnectNamedPipe(self.file.as_raw_handle() as HANDLE);
        }
    }
}

/// 管道服务端：每次 `accept` 创建实例并阻塞等待一个客户端。
pub struct Server {
    name: Vec<u16>,
}

impl Server {
    pub fn new(name: &str) -> Self {
        Self { name: wide(name) }
    }

    pub fn accept(&self) -> Result<Connection, String> {
        unsafe {
            let handle = CreateNamedPipeW(
                self.name.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                std::ptr::null(),
            );
            if handle == INVALID_HANDLE_VALUE {
                return Err(format!("CreateNamedPipeW failed: {}", GetLastError()));
            }
            // ERROR_PIPE_CONNECTED 表示客户端已先一步连上，属于正常情况。
            if ConnectNamedPipe(handle, std::ptr::null_mut()) == 0
                && GetLastError() != ERROR_PIPE_CONNECTED
            {
                CloseHandle(handle);
                return Err(format!("ConnectNamedPipe failed: {}", GetLastError()));
            }
            let file = File::from_raw_handle(handle as RawHandle);
            Ok(Connection {
                file,
                buf: Vec::with_capacity(4096),
            })
        }
    }
}

/// 客户端：在 `retry` 时间内反复尝试连接。
pub fn connect(name: &str, retry: std::time::Duration) -> Result<Connection, String> {
    let name = wide(name);
    let deadline = std::time::Instant::now() + retry;
    loop {
        unsafe {
            let handle = CreateFileW(
                name.as_ptr(),
                FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            );
            if handle != INVALID_HANDLE_VALUE {
                let file = File::from_raw_handle(handle as RawHandle);
                return Ok(Connection {
                    file,
                    buf: Vec::with_capacity(4096),
                });
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err("无法连接到 Spotlight 守护进程".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
