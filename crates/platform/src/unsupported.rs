//! 非 Windows 平台的占位实现。
//!
//! macOS 的目标是真实实现（见 `design-docs/01-形态与架构.md` §2.1），
//! 目前只是为了让 workspace 在别的平台上也能 `cargo check`。

pub fn unsupported() -> ! {
    unimplemented!("spotlight 目前只实现了 Windows 平台")
}
