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

/// 从名字构建效果链。未知名字忽略（前向兼容）；全部未知时退回默认光环。
pub fn build_effects(names: &[String]) -> Vec<Box<dyn Effect>> {
    if names.is_empty() {
        return vec![Box::new(Ring::default())];
    }

    let effects: Vec<Box<dyn Effect>> = names
        .iter()
        .filter_map(|name| match name.as_str() {
            "ring" => Some(Box::new(Ring::default()) as Box<dyn Effect>),
            "brush" => Some(Box::new(Brush::default()) as Box<dyn Effect>),
            _ => None,
        })
        .collect();

    if effects.is_empty() {
        vec![Box::new(Ring::default())]
    } else {
        effects
    }
}

/// 已知效果名，用于错误提示与文档。
pub const KNOWN_EFFECTS: &[&str] = &["ring", "brush"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Ctx, render_frame};

    fn solid_alpha(buf: &[u8], w: u32, x: u32, y: u32) -> u8 {
        buf[((y * w + x) * 4 + 3) as usize]
    }

    #[test]
    fn unknown_names_fall_back_to_ring() {
        let effects = build_effects(&["nope".to_string()]);
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].name(), "ring");
    }

    #[test]
    fn explicit_names_replace_default() {
        let effects = build_effects(&["brush".to_string()]);
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].name(), "brush");
    }

    #[test]
    fn ring_paints_outside_the_target() {
        let target = RectF::new(20.0, 20.0, 60.0, 30.0);
        let effects = build_effects(&[]);
        let ctx = Ctx::default();
        let buf = render_frame(100, 80, &[target], &effects, &ctx).unwrap();
        // 目标内部应当是空的
        assert_eq!(solid_alpha(&buf, 100, 50, 35), 0);
        // 目标外、光环中心线上应当是实心的
        assert!(solid_alpha(&buf, 100, 50, 17) > 150);
    }

    #[test]
    fn brush_draws_a_closed_loop_around_target() {
        let target = RectF::new(40.0, 40.0, 60.0, 40.0);
        let effects = build_effects(&["brush".to_string()]);
        let ctx = Ctx::default();
        let buf = render_frame(160, 140, &[target], &effects, &ctx).unwrap();

        // 圈上至少有一批不透明像素，且中间是空的
        let painted = buf.chunks_exact(4).filter(|px| px[3] > 100).count();
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
