//! Spotlight 命令行入口。

mod args;
mod client;
mod config;
mod daemon;
mod paths;
mod probe;

use std::process::exit;
use std::time::Instant;

use serde_json::json;
use spotlight_core::protocol::{
    CaptureParams, ClearParams, Method, Request, Response, ShowParams, TargetSpec, codes,
};
use spotlight_platform::win::{enable_dpi_awareness, primary_scale, screen_size};

fn main() {
    // 必须在任何窗口/坐标读取之前：否则 125% 缩放下所有坐标都会被虚拟化。
    enable_dpi_awareness();

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = argv.first() else {
        usage();
        exit(2);
    };

    let code = match command.as_str() {
        "daemon" => daemon::run(),
        "probe" => probe::run(),
        "show" => cmd_show(&argv[1..]),
        "capture" => cmd_capture(&argv[1..]),
        "clear" => cmd_clear(&argv[1..]),
        "status" => cmd_status(&argv[1..]),
        "doctor" => cmd_doctor(&argv[1..]),
        "help" | "-h" | "--help" => {
            usage();
            0
        }
        "version" | "-V" | "--version" => {
            println!("spotlight {}", spotlight_core::VERSION);
            0
        }
        other => {
            eprintln!("未知命令: {other}\n");
            usage();
            2
        }
    };
    exit(code);
}

fn usage() {
    println!(
        "spotlight <命令> [参数]

命令:
  capture  截一张图并返回坐标映射
  show     高亮一个区域
  clear    清除当前画面
  status   查看守护进程与当前画面
  doctor   自检（屏幕、DPI、守护进程、热键）
  daemon   前台运行守护进程（内部用）
  probe    技术验证探针

capture 参数:
  --max-width <像素>   长边上限，默认 1280
  --monitor <序号>     只截某块屏，默认主屏
  --json               机器可读输出

show 参数:
  --rect x,y,w,h       目标矩形（可重复给多个目标）
  --effect <名字>      效果：ring（默认）/ brush，可重复
  --capture <id>       坐标来自哪张截图（推荐）
  --space screen       或：坐标已经是屏幕物理像素
  --text <文案>        显示在卡片里
  --title <标题>       卡片标题
  --step <如 1/3>      步骤标记；可重复，按序对应 --rect
  --timeout <秒|auto>  0 表示常驻；缺省按字数估算
  --key <id>           相同 key 原地替换
  --json               机器可读输出"
    );
}

fn cmd_show(rest: &[String]) -> i32 {
    let parsed = match args::parse(rest) {
        Ok(p) => p,
        Err(err) => return usage_error(&err),
    };

    if parsed.all("rect").is_empty() {
        return usage_error("show 需要 --rect x,y,w,h（可重复）");
    }

    // --step 可以给多次，按顺序对应 --rect。
    let steps = parsed.all("step");
    let mut targets = Vec::new();
    for (index, raw) in parsed.all("rect").iter().enumerate() {
        match args::parse_rect(raw) {
            Ok(rect) => targets.push(TargetSpec {
                rect,
                step: steps.get(index).cloned(),
            }),
            Err(err) => return usage_error(&err),
        }
    }

    let timeout = match parsed.value("timeout") {
        None | Some("auto") => None,
        Some(raw) => match raw.parse::<f32>() {
            Ok(secs) => Some(secs),
            Err(_) => return usage_error("--timeout 需要秒数或 auto"),
        },
    };

    let params = ShowParams {
        rect: None,
        targets,
        capture: parsed.value("capture").map(str::to_string),
        space: parsed.value("space").map(str::to_string),
        text: parsed.value("text").map(str::to_string),
        title: parsed.value("title").map(str::to_string),
        step: None,
        effects: parsed.all("effect").to_vec(),
        timeout,
        key: parsed
            .value("key")
            .or(parsed.value("id"))
            .map(str::to_string),
    };

    let req = Request::new(1, Method::Show, Some(json!(params)));
    dispatch(req, parsed.flag("json"))
}

fn cmd_capture(rest: &[String]) -> i32 {
    let parsed = match args::parse(rest) {
        Ok(p) => p,
        Err(err) => return usage_error(&err),
    };
    let max_width = match parsed.value("max-width") {
        None => None,
        Some(raw) => match raw.parse::<u32>() {
            Ok(v) => Some(v),
            Err(_) => return usage_error("--max-width 需要正整数"),
        },
    };
    let monitor = match parsed.value("monitor") {
        None => None,
        Some(raw) => match raw.parse::<u32>() {
            Ok(v) => Some(v),
            Err(_) => return usage_error("--monitor 需要非负整数"),
        },
    };

    let params = CaptureParams { max_width, monitor };
    let req = Request::new(1, Method::Capture, Some(json!(params)));
    dispatch(req, parsed.flag("json"))
}

fn cmd_clear(rest: &[String]) -> i32 {
    let parsed = match args::parse(rest) {
        Ok(p) => p,
        Err(err) => return usage_error(&err),
    };
    let params = ClearParams {
        all: !parsed.flag("key"),
        key: parsed.value("key").map(str::to_string),
    };
    let req = Request::new(1, Method::Clear, Some(json!(params)));
    dispatch(req, parsed.flag("json"))
}

fn cmd_status(rest: &[String]) -> i32 {
    let parsed = match args::parse(rest) {
        Ok(p) => p,
        Err(err) => return usage_error(&err),
    };
    let req = Request::new(1, Method::Status, None);
    dispatch(req, parsed.flag("json"))
}

/// 自检：把"出问题时最需要知道的几件事"一次报出来。
fn cmd_doctor(rest: &[String]) -> i32 {
    let parsed = match args::parse(rest) {
        Ok(p) => p,
        Err(err) => return usage_error(&err),
    };

    let (screen_w, screen_h) = screen_size();
    let scale = primary_scale();
    let home = paths::data_home();
    let loaded = config::load(&paths::config_path());

    let started = Instant::now();
    let response = client::send(&Request::new(1, Method::Status, None));
    let ping_ms = started.elapsed().as_millis();

    let reachable = matches!(&response, Ok(r) if r.ok);
    let result = response.as_ref().ok().and_then(|r| r.result.as_ref());
    let daemon_version = result
        .and_then(|r| r.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    let hotkey = result
        .and_then(|r| r.get("hotkey"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let has_scene = result
        .and_then(|r| r.get("scene"))
        .is_some_and(|v| !v.is_null());

    if parsed.flag("json") {
        println!(
            "{}",
            json!({
                "ok": reachable,
                "version": spotlight_core::VERSION,
                "screen": { "width": screen_w, "height": screen_h, "scale": scale },
                "home": home.display().to_string(),
                "config": {
                    "path": loaded.path.display().to_string(),
                    "exists": loaded.exists,
                    "note": loaded.note,
                },
                "daemon": {
                    "reachable": reachable,
                    "version": daemon_version,
                    "hotkey": hotkey,
                    "ping_ms": ping_ms,
                }
            })
        );
        return 0;
    }

    println!("spotlight {}", spotlight_core::VERSION);
    println!("屏幕      : {screen_w}×{screen_h} @ {:.0}%", scale * 100.0);
    println!("数据目录  : {}", home.display());
    println!("配置      : {}", loaded.path.display());
    match (&loaded.note, loaded.exists) {
        (Some(note), _) => println!("            ⚠ {note}（该项回退默认值）"),
        (None, false) => println!("            （未找到，全部使用默认值）"),
        (None, true) => println!("            （已加载）"),
    }

    if reachable {
        println!("守护进程  : 正常（{ping_ms} ms，v{daemon_version}）");
        if has_scene {
            println!(
                "关闭热键  : {}",
                if hotkey {
                    "已注册（Ctrl+Alt+Esc）"
                } else {
                    "注册失败，可能被其他程序占用；画面仍可用关闭按钮关掉"
                }
            );
        } else {
            println!("关闭热键  : 空闲不占用，出现画面时才注册");
        }
    } else {
        println!("守护进程  : 连不上");
        match &response {
            Ok(resp) => {
                if let Some(err) = &resp.error {
                    println!("错误      : [{}] {}", err.code, err.message);
                }
            }
            Err(err) => println!("错误      : {err}"),
        }
    }
    0
}
fn usage_error(message: &str) -> i32 {
    eprintln!("参数错误: {message}");
    2
}

fn dispatch(req: Request, json_out: bool) -> i32 {
    match client::send(&req) {
        Ok(resp) => report(resp, json_out),
        Err(err) => {
            if json_out {
                println!(
                    "{}",
                    json!({"ok": false, "error": {"code": "daemon_unavailable", "message": err}})
                );
            } else {
                eprintln!("无法连接守护进程: {err}");
            }
            4
        }
    }
}

fn report(resp: Response, json_out: bool) -> i32 {
    if json_out {
        println!("{}", serde_json::to_string(&resp).unwrap_or_default());
    }

    if resp.ok {
        if !json_out {
            human_ok(&resp);
        }
        return 0;
    }

    let Some(err) = resp.error else {
        return 1;
    };
    if !json_out {
        eprintln!("错误 [{}]: {}", err.code, err.message);
        if let Some(hint) = err.hint {
            eprintln!("提示: {hint}");
        }
    }
    exit_code_for(&err.code)
}

fn human_ok(resp: &Response) {
    let Some(result) = &resp.result else {
        println!("ok");
        return;
    };

    if let Some(ms) = result.get("timeout_ms").and_then(|v| v.as_u64()) {
        println!("已显示，{:.1} 秒后自动消失", ms as f64 / 1000.0);
    } else if let Some(capture) = result.get("capture").and_then(|v| v.as_str()) {
        let image = result.get("image");
        let path = image
            .and_then(|i| i.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let width = image
            .and_then(|i| i.get("width"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let height = image
            .and_then(|i| i.get("height"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        println!("截图 {capture}（{width}×{height}）: {path}");
        println!("下一步: spotlight show --capture {capture} --rect x,y,w,h --text \"点这里\"");
    } else if let Some(scene) = result.get("scene").and_then(|v| v.as_str()) {
        println!("已显示 {scene}");
    } else if result.get("cleared").is_some() {
        println!("已清除");
    } else if result.get("alive").is_some() {
        human_status(result);
    } else {
        println!("ok");
    }
}

fn human_status(result: &serde_json::Value) {
    let version = result
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let uptime = result
        .get("uptime_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    println!("守护进程运行中 v{version}，已运行 {} 秒", uptime / 1000);

    match result.get("scene") {
        Some(scene) if !scene.is_null() => {
            let has_text = scene
                .get("has_text")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let remaining = scene
                .get("remaining_ms")
                .and_then(|v| v.as_u64())
                .map(|ms| format!("，剩余 {:.1} 秒", ms as f64 / 1000.0))
                .unwrap_or_else(|| "，常驻".to_string());
            println!(
                "当前画面: {}{remaining}",
                if has_text { "带文案" } else { "仅高亮" }
            );
        }
        _ => println!("当前没有画面"),
    }
}

fn exit_code_for(code: &str) -> i32 {
    match code {
        codes::BAD_PARAMS | codes::BAD_JSON => 2,
        codes::RECT_OUT_OF_BOUNDS => 3,
        codes::UNSUPPORTED_VERSION | codes::TIMEOUT => 4,
        codes::NO_SUCH_KEY => 5,
        codes::SPACE_UNSPECIFIED | codes::UNKNOWN_CAPTURE | codes::CAPTURE_FAILED => 3,
        _ => 1,
    }
}
