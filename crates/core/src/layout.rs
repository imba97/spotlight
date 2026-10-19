//! 文案卡片的摆放：八个候选方位 + 可用性打分。
//!
//! 规则见 `design-docs/03-渲染与视觉.md` §4：必须完整落在 work area 内、不与目标重叠，
//! 同分时按固定优先级取。都放不下时选溢出最小的，并夹进 work area。

use crate::geom::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub w: i32,
    pub h: i32,
}

impl Size {
    pub const fn new(w: i32, h: i32) -> Self {
        Self { w, h }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// 优先级顺序，也是同分时的取舍顺序。
const ORDER: [Placement; 8] = [
    Placement::Bottom,
    Placement::Top,
    Placement::Right,
    Placement::Left,
    Placement::BottomRight,
    Placement::BottomLeft,
    Placement::TopRight,
    Placement::TopLeft,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub rect: Rect,
    pub placement: Placement,
    /// 是否完整落在 work area 内
    pub fits: bool,
    /// 是否不可避免地盖住了目标
    pub overlaps_target: bool,
}

pub fn candidate(target: Rect, size: Size, gap: i32, placement: Placement) -> Rect {
    let centered_x = target.x + (target.w - size.w) / 2;
    let centered_y = target.y + (target.h - size.h) / 2;
    match placement {
        Placement::Bottom => Rect::new(centered_x, target.bottom() + gap, size.w, size.h),
        Placement::Top => Rect::new(centered_x, target.y - gap - size.h, size.w, size.h),
        Placement::Right => Rect::new(target.right() + gap, centered_y, size.w, size.h),
        Placement::Left => Rect::new(target.x - gap - size.w, centered_y, size.w, size.h),
        Placement::BottomLeft => Rect::new(target.x, target.bottom() + gap, size.w, size.h),
        Placement::BottomRight => Rect::new(
            target.right() - size.w,
            target.bottom() + gap,
            size.w,
            size.h,
        ),
        Placement::TopLeft => Rect::new(target.x, target.y - gap - size.h, size.w, size.h),
        Placement::TopRight => Rect::new(
            target.right() - size.w,
            target.y - gap - size.h,
            size.w,
            size.h,
        ),
    }
}

pub fn place(target: Rect, size: Size, work: Rect, gap: i32) -> Placed {
    // 1. 找一个既在 work 内、又不压住目标的方位，按优先级取第一个。
    //    候选框先沿垂直轴贴回 work，这样贴着屏幕边缘的目标也能把卡片放在正下方，
    //    而不是被迫换到侧面（tooltip 的常见行为）。
    for placement in ORDER {
        let rect = candidate(target, size, gap, placement).shift_into(work);
        if work.contains_rect(rect) && !rect.intersects(target) {
            return Placed {
                rect,
                placement,
                fits: true,
                overlaps_target: false,
            };
        }
    }

    // 2. 都不合适：取溢出面积最小的，再夹进 work。
    let mut best: Option<(i64, Placement, Rect)> = None;
    for placement in ORDER {
        let rect = candidate(target, size, gap, placement);
        let overflow = rect.outside_area(work);
        if best.is_none_or(|(o, _, _)| overflow < o) {
            best = Some((overflow, placement, rect));
        }
    }
    let (_, placement, rect) = best.expect("ORDER 非空");
    let clamped = rect.clamp_into(work);
    Placed {
        rect: clamped,
        placement,
        fits: false,
        overlaps_target: clamped.intersects(target),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect::new(0, 0, 1920, 1080);
    const SIZE: Size = Size::new(300, 100);

    #[test]
    fn centered_target_prefers_bottom() {
        let target = Rect::new(860, 490, 200, 100);
        let p = place(target, SIZE, WORK, 8);
        assert_eq!(p.placement, Placement::Bottom);
        assert!(p.fits);
        assert!(!p.overlaps_target);
        assert_eq!(p.rect.y, target.bottom() + 8);
        assert!(WORK.contains_rect(p.rect));
    }

    #[test]
    fn target_near_bottom_moves_to_top() {
        let target = Rect::new(860, 1040, 200, 30);
        let p = place(target, SIZE, WORK, 8);
        assert_eq!(p.placement, Placement::Top);
        assert!(p.fits);
        assert!(WORK.contains_rect(p.rect));
    }

    #[test]
    fn target_near_right_prefers_bottom_over_right() {
        // 右边放不下，下面能放下
        let target = Rect::new(1870, 400, 40, 40);
        let p = place(target, SIZE, WORK, 8);
        assert_eq!(p.placement, Placement::Bottom);
        assert!(p.fits);
    }

    #[test]
    fn never_overlaps_target_when_it_fits() {
        for tx in (0..1920).step_by(160) {
            for ty in (0..1080).step_by(120) {
                let target = Rect::new(tx, ty, 48, 24);
                let p = place(target, SIZE, WORK, 8);
                if p.fits {
                    assert!(!p.overlaps_target, "在 ({tx},{ty}) 处压住了目标");
                    assert!(WORK.contains_rect(p.rect));
                }
            }
        }
    }

    #[test]
    fn tiny_work_area_falls_back_and_clamps() {
        let work = Rect::new(0, 0, 320, 200);
        let target = Rect::new(140, 90, 40, 20);
        let p = place(target, SIZE, work, 8);
        assert!(!p.fits);
        assert!(work.contains_rect(p.rect), "兜底结果也必须落在 work 内");
    }

    #[test]
    fn gap_is_respected_on_every_side() {
        let target = Rect::new(800, 400, 100, 50);
        for placement in ORDER {
            let r = candidate(target, SIZE, 12, placement);
            // 候选框与目标的距离：要么贴边 12px，要么横向/纵向有重叠但另一边相隔 12
            let horizontal_gap = (r.x - target.right()).max(target.x - r.right());
            let vertical_gap = (r.y - target.bottom()).max(target.y - r.bottom());
            assert!(
                horizontal_gap >= 12 || vertical_gap >= 12,
                "{placement:?} 间距不足: {r:?}"
            );
        }
    }
}
