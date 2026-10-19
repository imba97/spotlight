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

/// 找到与矩形**重叠面积最大**的显示器。
///
/// 用重叠面积而不是"包含"，是因为目标可能跨屏或压在边界上；这样总能选出一个最相关的，
/// 而不是直接失败。
pub fn monitor_for_rect(monitors: &[Monitor], rect: Rect) -> Option<usize> {
    monitors
        .iter()
        .enumerate()
        .filter_map(|(index, monitor)| {
            rect.intersect(monitor.bounds)
                .map(|overlap| (overlap.area(), index))
        })
        .max_by_key(|(area, _)| *area)
        .map(|(_, index)| index)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_monitor_by_largest_overlap() {
        let monitors = vec![
            Monitor::new(
                MonitorId(0),
                "left",
                Rect::new(-1920, 0, 1920, 1080),
                Rect::new(-1920, 0, 1920, 1040),
                1.0,
                false,
            ),
            Monitor::new(
                MonitorId(1),
                "right",
                Rect::new(0, 0, 1920, 1080),
                Rect::new(0, 0, 1920, 1040),
                1.25,
                true,
            ),
        ];

        // 完全在左屏（负坐标）
        assert_eq!(
            monitor_for_rect(&monitors, Rect::new(-500, 100, 200, 50)),
            Some(0)
        );
        // 完全在右屏
        assert_eq!(
            monitor_for_rect(&monitors, Rect::new(100, 100, 200, 50)),
            Some(1)
        );
        // 跨屏：大部分在右屏
        assert_eq!(
            monitor_for_rect(&monitors, Rect::new(-100, 100, 400, 50)),
            Some(1)
        );
        // 完全在屏幕之外
        assert_eq!(
            monitor_for_rect(&monitors, Rect::new(5000, 5000, 10, 10)),
            None
        );
    }
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
