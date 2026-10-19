//! 客户端：连守护进程，连不上就拉起来再连。

use std::time::Duration;

use spotlight_core::protocol::{Request, Response};
use spotlight_platform::win::ipc;
use spotlight_platform::win::process::spawn_detached;

/// 守护进程通常已经在跑，先快速试一次。
const CONNECT_FAST: Duration = Duration::from_millis(30);
/// 刚拉起来时需要等它把管道建好。
const CONNECT_AFTER_SPAWN: Duration = Duration::from_millis(1500);

pub fn send(req: &Request) -> Result<Response, String> {
    let name = ipc::pipe_name();

    let mut conn = match ipc::connect(&name, CONNECT_FAST) {
        Ok(conn) => conn,
        Err(_) => {
            let exe = std::env::current_exe().map_err(|e| format!("无法定位自身路径: {e}"))?;
            spawn_detached(&exe, &["daemon"]).map_err(|e| format!("无法启动守护进程: {e}"))?;
            ipc::connect(&name, CONNECT_AFTER_SPAWN).map_err(|e| format!("守护进程未就绪: {e}"))?
        }
    };

    let line = serde_json::to_string(req).map_err(|e| e.to_string())?;
    conn.send(&line)?;
    let raw = conn
        .recv()?
        .ok_or_else(|| "守护进程没有返回结果".to_string())?;
    serde_json::from_str(&raw).map_err(|e| format!("响应解析失败: {e}"))
}
