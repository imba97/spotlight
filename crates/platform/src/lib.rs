//! 平台层。
//!
//! 这里是**唯一**允许出现 `unsafe` 的地方。对上层只暴露几个"你要什么"的方法，
//! 不透露怎么实现（见 `design-docs/08-开发大纲.md` §4）。

#[cfg(windows)]
pub mod win;

#[cfg(not(windows))]
pub mod unsupported;
