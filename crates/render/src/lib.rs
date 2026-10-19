//! 光栅化层：把一帧画成**预乘 BGRA** 位图，交给平台层提交。
//!
//! 一帧 = 卡片（可选）+ 每个目标上的效果链 + 文字（可选）+ 关闭叉（可选）。
//! 这一层不碰窗口、不碰屏幕，是纯函数，所以可以在没有桌面的环境里做快照测试。

mod effect;

pub use effect::{Brush, Effect, EffectStyle, KNOWN_EFFECTS, Ring, build_effects};

use spotlight_core::geom::RectF;
use tiny_skia::{LineCap, Paint, Path, PathBuilder, Pixmap, Rect as SkRect, Stroke, Transform};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Card {
    pub rect: RectF,
    pub radius: f32,
    /// RGBA
    pub bg: [u8; 4],
}

/// 卡片底部的倒计时进度条，`value` 从 1 递减到 0。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    pub value: f32,
    pub color: [u8; 4],
}

/// 卡片右上角的关闭叉。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CloseMark {
    pub rect: RectF,
    pub color: [u8; 4],
}

/// 一张已经排版好的 8 位 alpha 掩膜。
#[derive(Clone, Copy, Debug)]
pub struct TextLayer<'a> {
    pub rect: RectF,
    pub mask: &'a [u8],
    pub mask_width: u32,
    pub mask_height: u32,
    /// RGBA
    pub color: [u8; 4],
}

/// 目标角上的步骤标记（"1"、"2"、"1/3"）。
#[derive(Clone, Copy, Debug)]
pub struct Badge<'a> {
    pub rect: RectF,
    pub radius: f32,
    /// RGBA
    pub bg: [u8; 4],
    pub text: TextLayer<'a>,
}

/// 与效果无关的附属元素。
#[derive(Clone, Copy, Debug, Default)]
pub struct Ctx<'a> {
    pub card: Option<Card>,
    /// 卡片标题行（小一号、半透明）。
    pub title: Option<TextLayer<'a>>,
    pub text: Option<TextLayer<'a>>,
    pub progress: Option<Progress>,
    pub close: Option<CloseMark>,
    pub badges: &'a [Badge<'a>],
}

/// 效果可以往这里画。坐标是效果窗口的客户区坐标。
pub struct Canvas<'a> {
    pixmap: &'a mut Pixmap,
}

impl Canvas<'_> {
    pub fn fill_round_rect(&mut self, rect: RectF, radius: f32, color: [u8; 4]) {
        let Some(path) = sk_round_rect(rect, radius) else {
            return;
        };
        let p = paint(color);
        self.pixmap.fill_path(
            &path,
            &p,
            tiny_skia::FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    pub fn stroke_round_rect(&mut self, rect: RectF, radius: f32, width: f32, color: [u8; 4]) {
        let Some(path) = sk_round_rect(rect, radius) else {
            return;
        };
        let stroke = Stroke {
            width,
            ..Stroke::default()
        };
        self.stroke_path(&path, &stroke, color);
    }

    pub fn stroke_path(&mut self, path: &Path, stroke: &Stroke, color: [u8; 4]) {
        let paint = paint(color);
        self.pixmap
            .stroke_path(path, &paint, stroke, Transform::identity(), None);
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }
}

fn paint(color: [u8; 4]) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color[0], color[1], color[2], color[3]);
    paint.anti_alias = true;
    paint
}

fn sk_round_rect(rect: RectF, radius: f32) -> Option<Path> {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return None;
    }
    let r = radius.clamp(0.0, rect.w.min(rect.h) / 2.0);
    let mut builder = PathBuilder::new();
    if r <= 0.0 {
        builder.push_rect(SkRect::from_xywh(rect.x, rect.y, rect.w, rect.h)?);
        return builder.finish();
    }
    // 四个圆角用三次贝塞尔近似 90° 圆弧
    const KAPPA: f32 = 0.552_284_8;
    let k = KAPPA * r;
    let (x, y) = (rect.x, rect.y);
    let (right, bottom) = (rect.right(), rect.bottom());
    builder.move_to(x + r, y);
    builder.line_to(right - r, y);
    builder.cubic_to(right - r + k, y, right, y + r - k, right, y + r);
    builder.line_to(right, bottom - r);
    builder.cubic_to(
        right,
        bottom - r + k,
        right - r + k,
        bottom,
        right - r,
        bottom,
    );
    builder.line_to(x + r, bottom);
    builder.cubic_to(x + r - k, bottom, x, bottom - r + k, x, bottom - r);
    builder.line_to(x, y + r);
    builder.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    builder.close();
    builder.finish()
}

/// 画一帧，返回预乘 BGRA（可直接交给 `UpdateLayeredWindow`）。
pub fn render_frame(
    width: u32,
    height: u32,
    targets: &[RectF],
    effects: &[Box<dyn Effect>],
    ctx: &Ctx<'_>,
    opacity: f32,
) -> Option<Vec<u8>> {
    if width == 0 || height == 0 {
        return None;
    }
    let mut pixmap = Pixmap::new(width, height)?;

    if let Some(card) = ctx.card {
        draw_card(&mut pixmap, &card);
    }
    if let (Some(card), Some(progress)) = (ctx.card, ctx.progress) {
        draw_progress(&mut pixmap, &card, &progress);
    }

    {
        let mut canvas = Canvas {
            pixmap: &mut pixmap,
        };
        for target in targets {
            for effect in effects {
                effect.draw(&mut canvas, *target, ctx);
            }
        }
    }

    for badge in ctx.badges {
        if let Some(path) = sk_round_rect(badge.rect, badge.radius) {
            let p = paint(badge.bg);
            pixmap.fill_path(
                &path,
                &p,
                tiny_skia::FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        blend_text(&mut pixmap, &badge.text);
    }

    if let Some(title) = ctx.title {
        blend_text(&mut pixmap, &title);
    }
    if let Some(text) = ctx.text {
        blend_text(&mut pixmap, &text);
    }
    if let Some(close) = ctx.close {
        draw_close(&mut pixmap, &close);
    }

    Some(to_premultiplied_bgra(pixmap.data(), opacity))
}

fn draw_card(pixmap: &mut Pixmap, card: &Card) {
    let Some(path) = sk_round_rect(card.rect, card.radius) else {
        return;
    };
    let p = paint(card.bg);
    pixmap.fill_path(
        &path,
        &p,
        tiny_skia::FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn draw_progress(pixmap: &mut Pixmap, card: &Card, progress: &Progress) {
    let value = progress.value.clamp(0.0, 1.0);
    if value <= 0.0 {
        return;
    }
    let inset = card.radius.max(2.0) * 0.8;
    let available = (card.rect.w - inset * 2.0).max(0.0);
    let width = available * value;
    if width < 0.5 {
        return;
    }
    let height = 2.0;
    let bar = RectF::new(
        card.rect.x + inset,
        card.rect.bottom() - height - 2.0,
        width,
        height,
    );
    let Some(path) = sk_round_rect(bar, height / 2.0) else {
        return;
    };
    let p = paint(progress.color);
    pixmap.fill_path(
        &path,
        &p,
        tiny_skia::FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn draw_close(pixmap: &mut Pixmap, close: &CloseMark) {
    let r = close.rect;
    if r.w < 4.0 || r.h < 4.0 {
        return;
    }
    let pad = r.w * 0.26;
    let mut builder = PathBuilder::new();
    builder.move_to(r.x + pad, r.y + pad);
    builder.line_to(r.x + r.w - pad, r.y + r.h - pad);
    builder.move_to(r.x + r.w - pad, r.y + pad);
    builder.line_to(r.x + pad, r.y + r.h - pad);
    let Some(path) = builder.finish() else {
        return;
    };
    let stroke = Stroke {
        width: 1.6,
        line_cap: LineCap::Round,
        ..Stroke::default()
    };
    let p = paint(close.color);
    pixmap.stroke_path(&path, &p, &stroke, Transform::identity(), None);
}

/// 把 alpha 掩膜以 `color` 合成进画布（预乘空间里做 over）。
fn blend_text(pixmap: &mut Pixmap, text: &TextLayer<'_>) {
    if text.mask.len() < (text.mask_width * text.mask_height) as usize {
        return;
    }
    let canvas_w = pixmap.width() as i32;
    let canvas_h = pixmap.height() as i32;
    let canvas_width = pixmap.width();
    let ox = text.rect.x.round() as i32;
    let oy = text.rect.y.round() as i32;
    let [cr, cg, cb, ca] = text.color;

    let data = pixmap.data_mut();
    for y in 0..text.mask_height as i32 {
        for x in 0..text.mask_width as i32 {
            let px = ox + x;
            let py = oy + y;
            if px < 0 || py < 0 || px >= canvas_w || py >= canvas_h {
                continue;
            }
            let coverage = text.mask[(y as u32 * text.mask_width + x as u32) as usize] as u32;
            if coverage == 0 {
                continue;
            }
            let sa = coverage * ca as u32 / 255;
            if sa == 0 {
                continue;
            }
            let idx = ((py as u32 * canvas_width + px as u32) * 4) as usize;
            let inv = 255 - sa;
            let src = |c: u8| (c as u32 * sa) / 255;
            data[idx] = (src(cr) + data[idx] as u32 * inv / 255) as u8;
            data[idx + 1] = (src(cg) + data[idx + 1] as u32 * inv / 255) as u8;
            data[idx + 2] = (src(cb) + data[idx + 2] as u32 * inv / 255) as u8;
            data[idx + 3] = (sa + data[idx + 3] as u32 * inv / 255) as u8;
        }
    }
}

/// tiny-skia 输出的是预乘 RGBA，Windows 要的是预乘 BGRA。
fn to_premultiplied_bgra(rgba: &[u8], opacity: f32) -> Vec<u8> {
    let k = opacity.clamp(0.0, 1.0);
    let mut out = rgba.to_vec();
    for px in out.chunks_exact_mut(4) {
        px.swap(0, 2);
        if k < 1.0 {
            // 预乘空间：四个通道一起缩放才不会破坏 alpha <= 通道值的关系
            for c in px.iter_mut() {
                *c = (*c as f32 * k).round() as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(buf: &[u8], w: u32, x: u32, y: u32) -> u8 {
        buf[((y * w + x) * 4 + 3) as usize]
    }

    #[test]
    fn card_is_filled_inside_and_clear_outside() {
        let ctx = Ctx {
            card: Some(Card {
                rect: RectF::new(16.0, 16.0, 32.0, 32.0),
                radius: 6.0,
                bg: [27, 27, 31, 240],
            }),
            ..Ctx::default()
        };
        let buf = render_frame(64, 64, &[], &[], &ctx, 1.0).unwrap();
        assert!(alpha_at(&buf, 64, 32, 32) > 200, "卡片内部应不透明");
        assert_eq!(alpha_at(&buf, 64, 2, 2), 0, "卡片外部应透明");
    }

    #[test]
    fn text_mask_is_blended() {
        let mask = vec![255u8; 4];
        let ctx = Ctx {
            text: Some(TextLayer {
                rect: RectF::new(10.0, 10.0, 2.0, 2.0),
                mask: &mask,
                mask_width: 2,
                mask_height: 2,
                color: [255, 255, 255, 255],
            }),
            ..Ctx::default()
        };
        let buf = render_frame(32, 32, &[], &[], &ctx, 1.0).unwrap();
        let idx = ((10 * 32 + 10) * 4) as usize;
        assert_eq!(buf[idx + 3], 255);
        assert_eq!(buf[idx], 255);
    }

    #[test]
    fn progress_bar_shrinks_with_value() {
        let card = Card {
            rect: RectF::new(10.0, 10.0, 200.0, 60.0),
            radius: 10.0,
            bg: [0, 0, 0, 255],
        };
        let full = Ctx {
            card: Some(card),
            progress: Some(Progress {
                value: 1.0,
                color: [255, 59, 48, 255],
            }),
            ..Ctx::default()
        };
        let bar_y = 10 + 60 - 2 - 2 + 1;
        let buf = render_frame(220, 80, &[], &[], &full, 1.0).unwrap();
        let idx = ((bar_y * 220 + 190) * 4) as usize;
        assert_eq!(buf[idx + 2], 255, "满格时右端是进度条的红色");

        let short = Ctx {
            progress: Some(Progress {
                value: 0.05,
                color: [255, 59, 48, 255],
            }),
            ..full
        };
        let buf = render_frame(220, 80, &[], &[], &short, 1.0).unwrap();
        assert_ne!(buf[idx + 2], 255, "快结束时右端回到卡片底色");
    }

    #[test]
    fn badge_paints_pill_under_text() {
        let mask = vec![255u8; 4];
        let badges = [Badge {
            rect: RectF::new(8.0, 8.0, 20.0, 20.0),
            radius: 10.0,
            bg: [0, 0, 255, 255],
            text: TextLayer {
                rect: RectF::new(10.0, 10.0, 2.0, 2.0),
                mask: &mask,
                mask_width: 2,
                mask_height: 2,
                color: [255, 255, 255, 255],
            },
        }];
        let ctx = Ctx {
            badges: &badges,
            ..Ctx::default()
        };
        let buf = render_frame(40, 40, &[], &[], &ctx, 1.0).unwrap();
        // 药丸内部是纯蓝（BGRA 里 B=255），说明背景画上了
        let pill = ((18 * 40 + 18) * 4) as usize;
        assert_eq!(buf[pill + 3], 255);
        assert_eq!(buf[pill], 255);
        // 文字位置被白色覆盖
        let glyph = ((10 * 40 + 10) * 4) as usize;
        assert_eq!(buf[glyph], 255);
        assert_eq!(buf[glyph + 1], 255);
        assert_eq!(buf[glyph + 2], 255);
    }
    #[test]
    fn opacity_scales_premultiplied_channels() {
        let ctx = Ctx {
            card: Some(Card {
                rect: RectF::new(0.0, 0.0, 32.0, 32.0),
                radius: 0.0,
                bg: [255, 255, 255, 255],
            }),
            ..Ctx::default()
        };
        let full = render_frame(32, 32, &[], &[], &ctx, 1.0).unwrap();
        let half = render_frame(32, 32, &[], &[], &ctx, 0.5).unwrap();
        let idx = ((16 * 32 + 16) * 4) as usize;
        assert_eq!(full[idx + 3], 255);
        assert!((i32::from(half[idx + 3]) - 127).abs() <= 2, "alpha 应减半");
        // 预乘空间里 RGB 必须跟着 alpha 一起缩，否则会出现"发亮的半透明"
        assert!((i32::from(half[idx]) - 127).abs() <= 2, "RGB 应同比例缩放");
    }

    #[test]
    fn degenerate_frame_is_rejected() {
        assert!(render_frame(0, 10, &[], &[], &Ctx::default(), 1.0).is_none());
    }
}
