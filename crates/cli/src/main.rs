//! Spotlight 命令行入口。

mod probe;

fn main() {
    let cmd = std::env::args().nth(1);

    match cmd.as_deref() {
        Some("probe") => std::process::exit(probe::run()),
        Some(other) => {
            eprintln!("未知命令: {other}");
            eprintln!("用法: spotlight probe");
            std::process::exit(2);
        }
        None => {
            eprintln!("用法: spotlight probe");
            std::process::exit(2);
        }
    }
}
