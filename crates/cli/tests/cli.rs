//! CLI 集成测试：直接跑编译出来的二进制。
//!
//! 只覆盖**不需要桌面会话、也不会拉起守护进程**的路径——参数解析、帮助、退出码。
//! 涉及窗口和截图的行为由单测 + 真机验证负责。

use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_spotlight"))
        .args(args)
        .output()
        .expect("无法执行 spotlight")
}

fn stdout_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn version_prints_semver() {
    let out = run(&["version"]);
    assert!(out.status.success());
    let text = stdout_of(&out);
    assert!(text.starts_with("spotlight "), "实际输出: {text}");
    assert!(text.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_lists_all_commands() {
    let out = run(&["--help"]);
    assert!(out.status.success());
    let text = stdout_of(&out);
    for command in ["capture", "captures", "show", "clear", "status", "doctor"] {
        assert!(text.contains(command), "帮助里缺少命令: {command}");
    }
    assert!(text.contains("--dry-run"), "帮助里缺少 --dry-run");
}

#[test]
fn unknown_command_exits_2() {
    let out = run(&["explode"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr_of(&out).contains("未知命令"));
}

#[test]
fn no_command_prints_usage() {
    let out = run(&[]);
    assert_eq!(out.status.code(), Some(2));
    // 用法打在 stdout 上，退出码 2 表示这是误用
    assert!(stdout_of(&out).contains("spotlight <命令>"));
}

#[test]
fn show_without_rect_is_rejected() {
    let out = run(&["show", "--text", "hi"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr_of(&out).contains("--rect"));
}

#[test]
fn malformed_rect_is_rejected_before_dispatch() {
    // 参数在本地就解析失败，不会去碰守护进程
    let out = run(&["show", "--space", "screen", "--rect", "1,2,3"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr_of(&out).contains("--rect"));
}

#[test]
fn bad_timeout_is_rejected() {
    let out = run(&[
        "show",
        "--space",
        "screen",
        "--rect",
        "1,2,3,4",
        "--timeout",
        "soon",
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr_of(&out).contains("--timeout"));
}

#[test]
fn missing_scene_file_is_reported() {
    let out = run(&["show", "--file", "definitely-not-here.json"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr_of(&out).contains("读取场景文件失败"));
}

#[test]
fn broken_scene_file_is_reported() {
    // 写在 target 下，受限沙箱里也可写
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("test-cli");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("broken.json");
    std::fs::write(&file, "{ not json").unwrap();

    let out = run(&["show", "--file", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr_of(&out).contains("解析失败"));

    let _ = std::fs::remove_file(&file);
}
