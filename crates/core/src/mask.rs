//! 遮罩分解：把「屏幕减去若干个洞」拆成互不重叠的矩形集合。
//!
//! 每个矩形对应一个纯色分层窗口，**不产生任何位图**，所以内存占用与分辨率无关。
//! 这是 `design-docs/03-渲染与视觉.md` §2 的落点。

use crate::geom::Rect;

/// 超过这个数量的矩形，合成开销就不划算了，调用方应退化为「整屏逐像素窗口」。
pub const MAX_MASK_RECTS: usize = 24;

#[derive(Clone, Debug, PartialEq)]
pub struct MaskPlan {
    pub rects: Vec<Rect>,
    /// 为真时，调用方必须改用整屏位图方案，忽略 `rects`。
    pub fallback_fullscreen: bool,
}

/// 屏幕减去若干洞，返回一组互不重叠、并集等于「屏幕 − 洞」的矩形。
///
/// 洞会被先裁进屏幕；空洞、屏幕外的洞自动忽略。
pub fn subtract(screen: Rect, holes: &[Rect]) -> Vec<Rect> {
    if screen.is_empty() {
        return Vec::new();
    }

    let holes: Vec<Rect> = holes
        .iter()
        .filter_map(|h| h.intersect(screen))
        .filter(|h| !h.is_empty())
        .collect();

    if holes.is_empty() {
        return vec![screen];
    }

    // 所有可能改变「哪一段被挖空」的 y 值。
    let mut ys: Vec<i32> = Vec::with_capacity(holes.len() * 2 + 2);
    ys.push(screen.y);
    ys.push(screen.bottom());
    for h in &holes {
        ys.push(h.y);
        ys.push(h.bottom());
    }
    ys.sort_unstable();
    ys.dedup();

    let mut out = Vec::new();

    for band in ys.windows(2) {
        let (y0, y1) = (band[0], band[1]);
        if y1 <= y0 {
            continue;
        }

        // 这一段里所有洞的 x 区间
        let mut spans: Vec<(i32, i32)> = holes
            .iter()
            .filter(|h| h.y < y1 && h.bottom() > y0)
            .map(|h| (h.x, h.right()))
            .collect();
        if spans.is_empty() {
            out.push(Rect::from_edges(screen.x, y0, screen.right(), y1));
            continue;
        }
        spans.sort_unstable();

        let mut merged: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
        for (a, b) in spans {
            match merged.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }

        // 取补集
        let mut cursor = screen.x;
        for (a, b) in merged {
            if a > cursor {
                out.push(Rect::from_edges(cursor, y0, a, y1));
            }
            cursor = cursor.max(b);
            if cursor >= screen.right() {
                break;
            }
        }
        if cursor < screen.right() {
            out.push(Rect::from_edges(cursor, y0, screen.right(), y1));
        }
    }

    out
}

/// 带兜底判断的版本。矩形太多时置 `fallback_fullscreen`。
pub fn plan(screen: Rect, holes: &[Rect]) -> MaskPlan {
    let rects = subtract(screen, holes);
    let fallback_fullscreen = rects.len() > MAX_MASK_RECTS;
    MaskPlan {
        rects,
        fallback_fullscreen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect::new(0, 0, 1920, 1080);

    fn no_overlap(rects: &[Rect]) {
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert_eq!(a.intersect(*b), None, "矩形重叠: {a:?} 与 {b:?}");
            }
        }
    }

    fn all_inside(rects: &[Rect], bounds: Rect) {
        for r in rects {
            assert!(bounds.contains_rect(*r), "矩形越界: {r:?}");
        }
    }

    /// 把矩形集合画进一张小位图，用于验证「覆盖率」。
    fn rasterize(screen: Rect, rects: &[Rect]) -> Vec<bool> {
        let mut grid = vec![false; (screen.w * screen.h) as usize];
        for r in rects {
            for y in r.y..r.bottom() {
                for x in r.x..r.right() {
                    if screen.contains_point(x, y) {
                        grid[((y - screen.y) * screen.w + (x - screen.x)) as usize] = true;
                    }
                }
            }
        }
        grid
    }

    #[test]
    fn no_holes_returns_whole_screen() {
        assert_eq!(subtract(SCREEN, &[]), vec![SCREEN]);
    }

    #[test]
    fn hole_in_middle_gives_four_rects() {
        let hole = Rect::new(800, 400, 100, 100);
        let rects = subtract(SCREEN, &[hole]);
        assert_eq!(rects.len(), 4);
        no_overlap(&rects);
        all_inside(&rects, SCREEN);
        for r in &rects {
            assert!(!r.intersects(hole));
        }
    }

    #[test]
    fn hole_at_corner_gives_two_rects() {
        let hole = Rect::new(0, 0, 100, 100);
        let rects = subtract(SCREEN, &[hole]);
        assert_eq!(rects.len(), 2);
        no_overlap(&rects);
    }

    #[test]
    fn hole_covering_everything_gives_nothing() {
        assert!(subtract(SCREEN, &[SCREEN]).is_empty());
        assert!(subtract(SCREEN, &[Rect::new(-100, -100, 5000, 5000)]).is_empty());
    }

    #[test]
    fn overlapping_holes_are_merged_in_effect() {
        let a = Rect::new(100, 100, 200, 200);
        let b = Rect::new(200, 200, 200, 200);
        let rects = subtract(SCREEN, &[a, b]);
        no_overlap(&rects);
        for r in &rects {
            assert!(!r.intersects(a));
            assert!(!r.intersects(b));
        }
    }

    #[test]
    fn holes_outside_screen_are_ignored() {
        let r = Rect::new(3000, 3000, 100, 100);
        assert_eq!(subtract(SCREEN, &[r]), vec![SCREEN]);
    }

    /// 性质测试：任意洞集合下，矩形集合必须恰好覆盖「屏幕 − 洞」。
    #[test]
    fn random_holes_cover_exactly_the_complement() {
        let screen = Rect::new(0, 0, 40, 30);
        let mut seed: u32 = 0x1234_5678;
        let mut rng = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as i32
        };

        for round in 0..200 {
            let count = (rng() % 5) as usize;
            let holes: Vec<Rect> = (0..count)
                .map(|_| {
                    let x = rng() % 50 - 5;
                    let y = rng() % 40 - 5;
                    Rect::new(x, y, rng() % 15 + 1, rng() % 10 + 1)
                })
                .collect();

            let rects = subtract(screen, &holes);
            no_overlap(&rects);
            all_inside(&rects, screen);

            let got = rasterize(screen, &rects);
            for y in 0..screen.h {
                for x in 0..screen.w {
                    let in_hole = holes
                        .iter()
                        .any(|h| h.contains_point(screen.x + x, screen.y + y));
                    let idx = (y * screen.w + x) as usize;
                    assert_eq!(
                        got[idx], !in_hole,
                        "round {round}: 像素 ({x},{y}) 覆盖错误，holes={holes:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn plan_flags_fallback_only_when_too_many_rects() {
        let single = plan(SCREEN, &[Rect::new(800, 400, 100, 100)]);
        assert!(!single.fallback_fullscreen);

        // 条状洞：每个洞切一段，矩形数会超过阈值
        let holes: Vec<Rect> = (0..40).map(|i| Rect::new(0, i * 20, 100, 10)).collect();
        let many = plan(SCREEN, &holes);
        assert!(many.rects.len() > MAX_MASK_RECTS);
        assert!(many.fallback_fullscreen);
    }
}
