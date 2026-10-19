//! 可插拔的视觉效果。
//!
//! 每个效果是"针对一个目标、画到什么画布上"的纯函数：给定目标和时间（暂为静态），
//! 产出一组绘制调用。加一个新效果 = 加一个 impl + 在 `build_effects` 里注册一行。
//!
//! 这和设计文档 `03-渲染与视觉.md` §3 的 DrawOp 思路一致，只是把"指令列表"换成了
//! "直接往 Canvas 上画"——少一层中间表示，效果代码更好读。

use tiny_skia::{LineJoin, PathBuilder, Stroke};

use crate::{Canvas, Ctx};

use spotlight_core::geom::RectF;

pub trait Effect {
    fn name(&self) -> &'static str;

    /// 效果会向目标外扩多少（物理像素），用于计算效果窗口的包围盒。
    fn margin(&self) -> f32;

    fn draw(&self, canvas: &mut Canvas<'_>, target: RectF, ctx: &Ctx<'_>);
}

/// 圆角描边光环。
#[derive(Clone, Copy, Debug)]
pub struct Ring {
    pub stroke: f32,
    pub radius: f32,
    pub color: [u8; 4],
}

impl Default for Ring {
    fn default() -> Self {
        Self {
            stroke: 3.0,
            radius: 10.0,
            color: [255, 59, 48, 255],
        }
    }
}

impl Effect for Ring {
    fn name(&self) -> &'static str {
        "ring"
    }

    fn margin(&self) -> f32 {
        self.stroke
    }

    fn draw(&self, canvas: &mut Canvas<'_>, target: RectF, _ctx: &Ctx<'_>) {
        let outer = target.inflate(self.stroke);
        canvas.stroke_round_rect(outer, self.radius, self.stroke, self.color);
    }
}

/// 柔和外发光：由外向内叠几层逐渐变浓的描边，形成光晕。
#[derive(Clone, Copy, Debug)]
pub struct Glow {
    pub spread: f32,
    pub width: f32,
    pub radius: f32,
    pub color: [u8; 4],
}

impl Default for Glow {
    fn default() -> Self {
        Self {
            spread: 9.0,
            width: 2.0,
            radius: 12.0,
            color: [255, 59, 48, 255],
        }
    }
}

impl Effect for Glow {
    fn name(&self) -> &'static str {
        "glow"
    }

    fn margin(&self) -> f32 {
        self.spread + self.width
    }

    fn draw(&self, canvas: &mut Canvas<'_>, target: RectF, _ctx: &Ctx<'_>) {
        const LAYERS: usize = 6;
        for layer in 0..LAYERS {
            let t = layer as f32 / (LAYERS - 1) as f32; // 0 = 最外圈
            let spread = self.spread * (1.0 - t);
            let alpha = self.color[3] as f32 * (0.06 + 0.11 * t);
            if alpha < 1.0 {
                continue;
            }
            let mut color = self.color;
            color[3] = alpha.round() as u8;
            canvas.stroke_round_rect(target.inflate(spread), self.radius, self.width, color);
        }
    }
}
/// 从卡片指向目标的连线，末端带箭头。
///
/// 单目标时卡片总是贴着目标，画出来只有一截短棍——所以距离太近会直接跳过。
/// 它的价值在多目标：一张卡片对应多个目标时，连起来才看得出"这张卡在说哪几处"。
#[derive(Clone, Copy, Debug)]
pub struct Arrow {
    pub width: f32,
    pub head: f32,
    /// 目标那端要停在光环外沿，不能戳进内容里。
    pub tip: f32,
    /// 连线最短长度；低于它就认为卡片已经贴着目标，不画。
    pub min_length: f32,
    pub max_bend: f32,
    pub color: [u8; 4],
}

impl Default for Arrow {
    fn default() -> Self {
        Self {
            width: 1.8,
            head: 9.0,
            tip: 8.0,
            min_length: 26.0,
            max_bend: 22.0,
            color: [255, 59, 48, 220],
        }
    }
}

impl Effect for Arrow {
    fn name(&self) -> &'static str {
        "arrow"
    }

    fn margin(&self) -> f32 {
        self.max_bend
    }

    fn draw(&self, canvas: &mut Canvas<'_>, target: RectF, ctx: &Ctx<'_>) {
        let Some(card) = ctx.card else { return };
        let card = card.rect;
        if card.w <= 0.0 || card.h <= 0.0 || overlaps(card, target) {
            return;
        }

        let target_center = (target.x + target.w / 2.0, target.y + target.h / 2.0);
        let card_center = (card.x + card.w / 2.0, card.y + card.h / 2.0);

        let start = edge_point(card, target_center);
        // 停在光环外沿，箭头不要戳到目标内容上
        let end = edge_point(target.inflate(self.tip), card_center);

        let dx = end.0 - start.0;
        let dy = end.1 - start.1;
        let length = (dx * dx + dy * dy).sqrt();
        if length < self.min_length {
            return;
        }

        // 控制点在中点垂直方向偏一点，看起来像手画的连线而不是尺子比出来的
        let (nx, ny) = (-dy / length, dx / length);
        let bend = (length * 0.10).min(self.max_bend);
        let control = (
            (start.0 + end.0) / 2.0 + nx * bend,
            (start.1 + end.1) / 2.0 + ny * bend,
        );

        let mut builder = PathBuilder::new();
        builder.move_to(start.0, start.1);
        builder.quad_to(control.0, control.1, end.0, end.1);

        // 箭头的两条倒刺，方向取曲线在末端的切线
        let tx = end.0 - control.0;
        let ty = end.1 - control.1;
        let tl = (tx * tx + ty * ty).sqrt();
        if tl > 0.01 {
            let (ux, uy) = (tx / tl, ty / tl);
            for sign in [-1.0_f32, 1.0] {
                let angle = 0.45 * sign;
                let (sin, cos) = angle.sin_cos();
                let bx = ux * cos - uy * sin;
                let by = ux * sin + uy * cos;
                builder.move_to(end.0, end.1);
                builder.line_to(end.0 - bx * self.head, end.1 - by * self.head);
            }
        }

        let Some(path) = builder.finish() else { return };
        let stroke = Stroke {
            width: self.width,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        canvas.stroke_path(&path, &stroke, self.color);
    }
}

fn overlaps(a: RectF, b: RectF) -> bool {
    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
}

/// 从矩形中心朝 `toward` 发一条射线，求它与矩形边界的交点。
fn edge_point(rect: RectF, toward: (f32, f32)) -> (f32, f32) {
    let cx = rect.x + rect.w / 2.0;
    let cy = rect.y + rect.h / 2.0;
    let dx = toward.0 - cx;
    let dy = toward.1 - cy;
    if dx.abs() < 0.001 && dy.abs() < 0.001 {
        return (cx, cy);
    }
    let tx = if dx.abs() < 0.001 {
        f32::INFINITY
    } else {
        (rect.w / 2.0) / dx.abs()
    };
    let ty = if dy.abs() < 0.001 {
        f32::INFINITY
    } else {
        (rect.h / 2.0) / dy.abs()
    };
    let t = tx.min(ty);
    (cx + dx * t, cy + dy * t)
}
/// 模拟手绘的圈。沿着目标外围画一个抖动的椭圆，笔画略有起伏。
#[derive(Clone, Copy, Debug)]
pub struct Brush {
    pub width: f32,
    /// 抖动幅度（物理像素）。
    pub jitter: f32,
    pub margin: f32,
    pub color: [u8; 4],
}

impl Default for Brush {
    fn default() -> Self {
        Self {
            width: 2.6,
            jitter: 2.6,
            margin: 10.0,
            color: [255, 59, 48, 255],
        }
    }
}

impl Effect for Brush {
    fn name(&self) -> &'static str {
        "brush"
    }

    fn margin(&self) -> f32 {
        self.margin + self.jitter
    }

    fn draw(&self, canvas: &mut Canvas<'_>, target: RectF, _ctx: &Ctx<'_>) {
        let Some(path) = hand_drawn_ellipse(target, self.margin, self.jitter) else {
            return;
        };
        let stroke = Stroke {
            width: self.width,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        canvas.stroke_path(&path, &stroke, self.color);
    }
}

/// 围绕目标的抖动闭合椭圆，用二次贝塞尔穿过采样点中点做平滑。
fn hand_drawn_ellipse(target: RectF, margin: f32, jitter: f32) -> Option<tiny_skia::Path> {
    const SAMPLES: usize = 20;

    let cx = target.x + target.w / 2.0;
    let cy = target.y + target.h / 2.0;
    let rx = target.w / 2.0 + margin;
    let ry = target.h / 2.0 + margin;
    if rx <= 0.0 || ry <= 0.0 {
        return None;
    }

    // 用目标位置做种子：同一块区域每次重绘的圈长得一样，不会"抖个不停"。
    let mut seed = (target.x as i32 as u32)
        .wrapping_mul(73_856_093)
        .wrapping_add((target.y as i32 as u32).wrapping_mul(19_349_663))
        ^ 0x9E37_79B9;

    let mut points = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        let angle = i as f32 / SAMPLES as f32 * std::f32::consts::TAU;
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = ((seed >> 8) as f32 / 16_777_216.0 - 0.5) * 2.0 * jitter;
        points.push((
            cx + angle.cos() * (rx + noise),
            cy + angle.sin() * (ry + noise),
        ));
    }

    let mid = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let start = mid(points[SAMPLES - 1], points[0]);

    let mut builder = PathBuilder::new();
    builder.move_to(start.0, start.1);
    for i in 0..SAMPLES {
        let current = points[i];
        let next = points[(i + 1) % SAMPLES];
        let end = mid(current, next);
        builder.quad_to(current.0, current.1, end.0, end.1);
    }
    builder.close();
    builder.finish()
}

/// 效果的外观参数，来自配置或默认值。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectStyle {
    pub accent: [u8; 4],
    pub ring_width: f32,
    pub radius: f32,
    /// 所在显示器的 DPI 缩放；所有尺寸都乘它。
    pub scale: f32,
}

impl Default for EffectStyle {
    fn default() -> Self {
        Self {
            accent: [255, 59, 48, 255],
            ring_width: 3.0,
            radius: 10.0,
            scale: 1.0,
        }
    }
}

/// 从名字构建效果链。未知名字忽略（前向兼容）；全部未知时退回默认光环。
pub fn build_effects(names: &[String], style: &EffectStyle) -> Vec<Box<dyn Effect>> {
    let effects: Vec<Box<dyn Effect>> = names
        .iter()
        .filter_map(|name| match name.as_str() {
            "ring" => Some(ring(style)),
            "arrow" => {
                let base = Arrow::default();
                Some(Box::new(Arrow {
                    width: base.width * style.scale,
                    head: base.head * style.scale,
                    tip: base.tip * style.scale,
                    min_length: base.min_length * style.scale,
                    max_bend: base.max_bend * style.scale,
                    color: style.accent,
                }) as Box<dyn Effect>)
            }
            "glow" => {
                let base = Glow::default();
                Some(Box::new(Glow {
                    spread: base.spread * style.scale,
                    width: base.width * style.scale,
                    radius: base.radius * style.scale,
                    color: style.accent,
                }) as Box<dyn Effect>)
            }
            "brush" => {
                let base = Brush::default();
                Some(Box::new(Brush {
                    width: base.width * style.scale,
                    jitter: base.jitter * style.scale,
                    margin: base.margin * style.scale,
                    color: style.accent,
                }) as Box<dyn Effect>)
            }
            _ => None,
        })
        .collect();

    if effects.is_empty() {
        vec![ring(style)]
    } else {
        effects
    }
}

fn ring(style: &EffectStyle) -> Box<dyn Effect> {
    Box::new(Ring {
        stroke: style.ring_width * style.scale,
        radius: style.radius * style.scale,
        color: style.accent,
    })
}

/// 已知效果名，用于错误提示与文档。
pub const KNOWN_EFFECTS: &[&str] = &["ring", "brush", "glow", "arrow"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Ctx, render_frame};

    fn solid_alpha(buf: &[u8], w: u32, x: u32, y: u32) -> u8 {
        buf[((y * w + x) * 4 + 3) as usize]
    }

    #[test]
    fn unknown_names_fall_back_to_ring() {
        let effects = build_effects(&["nope".to_string()], &EffectStyle::default());
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].name(), "ring");
    }

    #[test]
    fn explicit_names_replace_default() {
        let effects = build_effects(&["brush".to_string()], &EffectStyle::default());
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].name(), "brush");
    }

    #[test]
    fn style_scale_multiplies_effect_metrics() {
        let base = EffectStyle::default();
        let doubled = EffectStyle { scale: 2.0, ..base };
        let ring_one = build_effects(&["ring".to_string()], &base);
        let ring_two = build_effects(&["ring".to_string()], &doubled);
        assert!((ring_two[0].margin() - ring_one[0].margin() * 2.0).abs() < 0.01);

        let brush_one = build_effects(&["brush".to_string()], &base);
        let brush_two = build_effects(&["brush".to_string()], &doubled);
        assert!((brush_two[0].margin() - brush_one[0].margin() * 2.0).abs() < 0.01);
    }
    #[test]
    fn arrow_connects_card_and_far_target() {
        let target = RectF::new(20.0, 20.0, 80.0, 40.0);
        let ctx = Ctx {
            card: Some(crate::Card {
                rect: RectF::new(20.0, 150.0, 200.0, 40.0),
                radius: 8.0,
                bg: [20, 20, 24, 230],
            }),
            ..Ctx::default()
        };
        let effects = build_effects(&["arrow".to_string()], &EffectStyle::default());
        let buf = render_frame(260, 220, &[target], &effects, &ctx, 1.0).unwrap();

        let mut painted = 0;
        for y in 65..145 {
            for x in 20..240 {
                let idx = ((y * 260 + x) * 4 + 3) as usize;
                if buf[idx] > 80 {
                    painted += 1;
                }
            }
        }
        assert!(painted > 40, "卡片与目标之间应该画出连线，实际 {painted}");
    }

    #[test]
    fn arrow_handles_horizontal_layout() {
        let target = RectF::new(200.0, 100.0, 60.0, 30.0);
        let ctx = Ctx {
            card: Some(crate::Card {
                rect: RectF::new(20.0, 100.0, 80.0, 30.0),
                radius: 6.0,
                bg: [0, 0, 0, 0],
            }),
            ..Ctx::default()
        };
        let effects = build_effects(&["arrow".to_string()], &EffectStyle::default());
        let buf = render_frame(300, 200, &[target], &effects, &ctx, 1.0).unwrap();
        let mut painted = 0;
        for y in 95..140 {
            for x in 100..200 {
                let idx = ((y * 300 + x) * 4 + 3) as usize;
                if buf[idx] > 80 {
                    painted += 1;
                }
            }
        }
        assert!(painted > 20, "水平方向也应该画连线，实际 {painted}");
    }
    #[test]
    fn arrow_skips_adjacent_card() {
        let target = RectF::new(20.0, 20.0, 80.0, 40.0);
        // 卡片紧贴目标（间距 < min_length），且背景透明，画了就会被看见
        let ctx = Ctx {
            card: Some(crate::Card {
                rect: RectF::new(20.0, 74.0, 200.0, 40.0),
                radius: 8.0,
                bg: [0, 0, 0, 0],
            }),
            ..Ctx::default()
        };
        let effects = build_effects(&["arrow".to_string()], &EffectStyle::default());
        let buf = render_frame(260, 220, &[target], &effects, &ctx, 1.0).unwrap();
        let painted = buf
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[3] > 40)
            .count();
        assert_eq!(painted, 0, "贴在一起时不该画箭头");
    }
    #[test]
    fn ring_paints_outside_the_target() {
        let target = RectF::new(20.0, 20.0, 60.0, 30.0);
        let effects = build_effects(&[], &EffectStyle::default());
        let ctx = Ctx::default();
        let buf = render_frame(100, 80, &[target], &effects, &ctx, 1.0).unwrap();
        // 目标内部应当是空的
        assert_eq!(solid_alpha(&buf, 100, 50, 35), 0);
        // 目标外、光环中心线上应当是实心的
        assert!(solid_alpha(&buf, 100, 50, 17) > 150);
    }

    #[test]
    fn brush_draws_a_closed_loop_around_target() {
        let target = RectF::new(40.0, 40.0, 60.0, 40.0);
        let effects = build_effects(&["brush".to_string()], &EffectStyle::default());
        let ctx = Ctx::default();
        let buf = render_frame(160, 140, &[target], &effects, &ctx, 1.0).unwrap();

        // 圈上至少有一批不透明像素，且中间是空的
        let painted = buf
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[3] > 100)
            .count();
        assert!(painted > 100, "应该画出笔画，实际 {painted}");
        assert_eq!(solid_alpha(&buf, 160, 70, 60), 0, "圈内应当透出背景");
    }

    #[test]
    fn brush_is_deterministic() {
        let target = RectF::new(10.0, 10.0, 40.0, 30.0);
        let a = hand_drawn_ellipse(target, 8.0, 3.0).unwrap();
        let b = hand_drawn_ellipse(target, 8.0, 3.0).unwrap();
        assert_eq!(a.bounds(), b.bounds());
        assert_eq!(a.segments().count(), b.segments().count());
    }
}
