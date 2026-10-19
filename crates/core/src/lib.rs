//! Spotlight 的纯逻辑层。
//!
//! 这个 crate 里**不允许**出现任何平台调用、`unsafe`、文件系统或网络访问。
//! 它只负责：几何、坐标换算、遮罩分解、卡片布局、时长估算、协议。
//! 平台相关的实现在 `spotlight-platform`，光栅化在 `spotlight-render`。

#![forbid(unsafe_code)]

pub mod geom;
pub mod layout;
pub mod mapping;
pub mod mask;
pub mod monitor;
pub mod protocol;
pub mod scene;
pub mod timing;

pub use geom::{ImageRect, Rect, RectF, ScreenRect};
pub use layout::{Placed, Placement, Size};
pub use mapping::{CaptureId, CaptureMap, MapError, Mapped, Segment};
pub use mask::{MAX_MASK_RECTS, MaskPlan};
pub use monitor::{Monitor, MonitorId};
pub use scene::{Callout, Scene, SceneError, Target};
pub use timing::{Timeout, estimate_timeout, resolve};

/// 与 Cargo 版本号一致，`--version` 会带上它。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
