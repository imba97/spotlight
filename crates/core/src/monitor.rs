//! 显示器描述。矩形一律是物理像素、虚拟桌面左上原点。

use crate::geom::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MonitorId(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub id: MonitorId,
    pub name: String,
    /// 整块屏幕。
    pub bounds: Rect,
    /// 去掉任务栏 / Dock 之后的可用区域。
    pub work: Rect,
    /// DPI 缩放，1.0 = 96 DPI。
    pub scale: f32,
    pub primary: bool,
}

impl Monitor {
    pub fn new(
        id: MonitorId,
        name: impl Into<String>,
        bounds: Rect,
        work: Rect,
        scale: f32,
        primary: bool,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            bounds,
            work,
            scale,
            primary,
        }
    }

    /// 物理像素 → DIP，用于让样式跟用户视觉大小一致。
    pub fn to_dip(&self, px: i32) -> f32 {
        px as f32 / self.scale.max(0.1)
    }

    /// DIP → 物理像素。
    pub fn from_dip(&self, dip: f32) -> i32 {
        (dip * self.scale.max(0.1)).round() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dip_conversion_round_trips_on_common_scales() {
        for scale in [1.0_f32, 1.25, 1.5, 2.0] {
            let m = Monitor::new(
                MonitorId(0),
                "test",
                Rect::new(0, 0, 1920, 1080),
                Rect::new(0, 0, 1920, 1040),
                scale,
                true,
            );
            assert_eq!(m.from_dip(m.to_dip(14)), 14, "scale {scale}");
        }
    }
}
