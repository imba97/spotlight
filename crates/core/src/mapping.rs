//! 截图像素 → 屏幕物理像素的映射。
//!
//! 这是整个项目里唯一允许做坐标换算的地方。契约见 `design-docs/02-坐标与截图.md`：
//! AI 在截图里量出矩形，用 `--capture <id>` 引用回来，由这里换算成屏幕坐标。

use crate::geom::{ImageRect, Rect, ScreenRect};
use crate::monitor::{Monitor, MonitorId};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CaptureId(pub String);

impl CaptureId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

impl std::fmt::Display for CaptureId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// 一张图里的一块对应屏幕上的哪一块。两边的矩形各自独立，
/// 不假设方向一致（显示器可以在任意方位）。
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub monitor: MonitorId,
    pub image: Rect,
    pub screen: Rect,
}

impl Segment {
    pub fn new(monitor: MonitorId, image: Rect, screen: Rect) -> Self {
        Self {
            monitor,
            image,
            screen,
        }
    }

    fn scale_x(&self) -> f64 {
        if self.image.w == 0 {
            0.0
        } else {
            f64::from(self.screen.w) / f64::from(self.image.w)
        }
    }

    fn scale_y(&self) -> f64 {
        if self.image.h == 0 {
            0.0
        } else {
            f64::from(self.screen.h) / f64::from(self.image.h)
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CaptureMap {
    pub id: CaptureId,
    pub image_size: (u32, u32),
    pub monitors: Vec<Monitor>,
    pub segments: Vec<Segment>,
}

/// 换算结果。`clipped` 表示原矩形跨出了图的边界，只取到了重叠部分。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapped {
    pub rect: ScreenRect,
    pub clipped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    /// 矩形为空。
    EmptyInput,
    /// 这张图没有任何分段信息。
    NoSegments,
    /// 矩形完全落在图外。
    Outside,
}

impl std::fmt::Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            MapError::EmptyInput => "输入矩形为空",
            MapError::NoSegments => "截图缺少坐标映射信息",
            MapError::Outside => "矩形完全落在截图范围之外",
        };
        f.write_str(s)
    }
}

impl std::error::Error for MapError {}

impl CaptureMap {
    /// 把截图像素矩形换算到屏幕物理像素。
    ///
    /// 跨分段时取**重叠面积最大**的那一段，并置 `clipped`。
    pub fn to_screen(&self, r: ImageRect) -> Result<Mapped, MapError> {
        let r = r.rect();
        if r.is_empty() {
            return Err(MapError::EmptyInput);
        }
        if self.segments.is_empty() {
            return Err(MapError::NoSegments);
        }

        let mut best: Option<(i64, &Segment, Rect)> = None;
        for seg in &self.segments {
            let Some(overlap) = r.intersect(seg.image) else {
                continue;
            };
            let area = overlap.area();
            if best.as_ref().is_none_or(|(a, _, _)| area > *a) {
                best = Some((area, seg, overlap));
            }
        }

        let Some((_, seg, overlap)) = best else {
            return Err(MapError::Outside);
        };

        let sx = seg.scale_x();
        let sy = seg.scale_y();

        // 先算两条边再相减，避免宽高各自四舍五入导致的 1px 漂移。
        let left = (f64::from(seg.screen.x) + f64::from(overlap.x - seg.image.x) * sx).round();
        let top = (f64::from(seg.screen.y) + f64::from(overlap.y - seg.image.y) * sy).round();
        let right =
            (f64::from(seg.screen.x) + f64::from(overlap.right() - seg.image.x) * sx).round();
        let bottom =
            (f64::from(seg.screen.y) + f64::from(overlap.bottom() - seg.image.y) * sy).round();

        let rect = Rect::from_edges(left as i32, top as i32, right as i32, bottom as i32);

        Ok(Mapped {
            rect: ScreenRect(rect),
            clipped: overlap != r,
        })
    }

    /// 找到包含某个屏幕点的显示器，用于「鼠标在哪个屏」之类的判断。
    pub fn monitor_at(&self, p: (i32, i32)) -> Option<&Monitor> {
        self.monitors
            .iter()
            .find(|m| m.bounds.contains_point(p.0, p.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(w: i32, h: i32, scale: f32) -> Monitor {
        Monitor::new(
            MonitorId(0),
            "m0",
            Rect::new(0, 0, w, h),
            Rect::new(0, 0, w, h),
            scale,
            true,
        )
    }

    fn map_1to1() -> CaptureMap {
        CaptureMap {
            id: CaptureId::new("c-1"),
            image_size: (1920, 1080),
            monitors: vec![monitor(1920, 1080, 1.0)],
            segments: vec![Segment::new(
                MonitorId(0),
                Rect::new(0, 0, 1920, 1080),
                Rect::new(0, 0, 1920, 1080),
            )],
        }
    }

    #[test]
    fn identity_mapping() {
        let m = map_1to1();
        let got = m.to_screen(ImageRect::new(10, 20, 30, 40)).unwrap();
        assert_eq!(got.rect, ScreenRect(Rect::new(10, 20, 30, 40)));
        assert!(!got.clipped);
    }

    #[test]
    fn downscaled_image_scales_up() {
        // 1280×720 的图对应 1920×1080 的屏，比例 1.5
        let m = CaptureMap {
            id: CaptureId::new("c-2"),
            image_size: (1280, 720),
            monitors: vec![monitor(1920, 1080, 1.25)],
            segments: vec![Segment::new(
                MonitorId(0),
                Rect::new(0, 0, 1280, 720),
                Rect::new(0, 0, 1920, 1080),
            )],
        };
        let got = m.to_screen(ImageRect::new(100, 100, 50, 50)).unwrap();
        assert_eq!(got.rect, ScreenRect(Rect::new(150, 150, 75, 75)));
    }

    #[test]
    fn negative_origin_screen_is_preserved() {
        let m = CaptureMap {
            id: CaptureId::new("c-3"),
            image_size: (1920, 1080),
            monitors: vec![Monitor::new(
                MonitorId(1),
                "left",
                Rect::new(-1920, 0, 1920, 1080),
                Rect::new(-1920, 0, 1920, 1080),
                1.0,
                false,
            )],
            segments: vec![Segment::new(
                MonitorId(1),
                Rect::new(0, 0, 1920, 1080),
                Rect::new(-1920, 0, 1920, 1080),
            )],
        };
        let got = m.to_screen(ImageRect::new(0, 0, 10, 10)).unwrap();
        assert_eq!(got.rect, ScreenRect(Rect::new(-1920, 0, 10, 10)));
    }

    #[test]
    fn rect_outside_image_is_an_error() {
        let m = map_1to1();
        assert_eq!(
            m.to_screen(ImageRect::new(5000, 5000, 10, 10)),
            Err(MapError::Outside)
        );
    }

    #[test]
    fn empty_rect_is_an_error() {
        let m = map_1to1();
        assert_eq!(
            m.to_screen(ImageRect::new(0, 0, 0, 10)),
            Err(MapError::EmptyInput)
        );
    }

    #[test]
    fn crossing_segments_picks_larger_overlap_and_clips() {
        // 两张屏并排拼成一张 3840 宽的图
        let m = CaptureMap {
            id: CaptureId::new("c-4"),
            image_size: (3840, 1080),
            monitors: vec![
                monitor(1920, 1080, 1.0),
                Monitor::new(
                    MonitorId(1),
                    "right",
                    Rect::new(1920, 0, 1920, 1080),
                    Rect::new(1920, 0, 1920, 1080),
                    1.0,
                    false,
                ),
            ],
            segments: vec![
                Segment::new(
                    MonitorId(0),
                    Rect::new(0, 0, 1920, 1080),
                    Rect::new(0, 0, 1920, 1080),
                ),
                Segment::new(
                    MonitorId(1),
                    Rect::new(1920, 0, 1920, 1080),
                    Rect::new(1920, 0, 1920, 1080),
                ),
            ],
        };
        // 大部分在右屏
        let got = m.to_screen(ImageRect::new(1800, 100, 400, 50)).unwrap();
        assert!(got.clipped);
        assert_eq!(got.rect, ScreenRect(Rect::new(1920, 100, 280, 50)));
    }

    #[test]
    fn no_segments_is_an_error() {
        let m = CaptureMap {
            id: CaptureId::new("c-5"),
            image_size: (10, 10),
            monitors: vec![],
            segments: vec![],
        };
        assert_eq!(
            m.to_screen(ImageRect::new(0, 0, 5, 5)),
            Err(MapError::NoSegments)
        );
    }
}
