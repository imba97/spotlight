//! 光栅化层：把一帧的矢量描述画成**预乘 BGRA** 位图，交给平台层提交。
//!
//! 这一层不碰窗口、不碰屏幕，是纯函数，所以可以在没有桌面的环境里做快照测试。

use spotlight_core::geom::RectF;
use tiny_skia::{LineCap, Paint, PathBuilder, Pixmap, Rect as SkRect, Stroke, Transform};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub rect: RectF,
    pub stroke: f32,
    pub radius: f32,
    /// RGBA
    pub color: [u8; 4],
}

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

#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    pub ring: Option<Ring>,
    pub card: Option<Card>,
    pub text: Option<TextLayer<'a>>,
    pub progress: Option<Progress>,
    pub close: Option<CloseMark>,
}

impl Frame<'_> {
    /// 只画一个框时的最小帧。
    pub fn empty(width: u32, height: u32) -> Frame<'static> {
        Frame {
            width,
            height,
            ring: None,
            card: None,
            text: None,
            progress: None,
            close: None,
        }
    }
}

/// 画一帧，返回预乘 BGRA（可直接交给 `UpdateLayeredWindow`）。
pub fn render_frame(spec: &Frame<'_>) -> Option<Vec<u8>> {
    if spec.width == 0 || spec.height == 0 {
        return None;
    }
    let mut pixmap = Pixmap::new(spec.width, spec.height)?;

    if let Some(card) = spec.card {
        draw_card(&mut pixmap, &card);
    }
    if let (Some(card), Some(progress)) = (spec.card, spec.progress) {
        draw_progress(&mut pixmap, &card, &progress);
    }
    if let Some(ring) = spec.ring {
        draw_ring(&mut pixmap, &ring);
    }
    if let Some(text) = spec.text {
        blend_text(&mut pixmap, &text);
    }
    if let Some(close) = spec.close {
        draw_close(&mut pixmap, &close);
    }

    Some(to_premultiplied_bgra(pixmap.data()))
}

fn round_rect_path(rect: SkRect, radius: f32) -> Option<tiny_skia::Path> {
    let (x, y, w, h) = (rect.x(), rect.y(), rect.width(), rect.height());
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = radius.clamp(0.0, w.min(h) / 2.0);
    let mut builder = PathBuilder::new();
    if r <= 0.0 {
        builder.push_rect(rect);
        return builder.finish();
    }
    // 四个圆角用三次贝塞尔近似 90° 圆弧
    const KAPPA: f32 = 0.552_284_8;
    let k = KAPPA * r;
    let (right, bottom) = (x + w, y + h);
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

fn draw_ring(pixmap: &mut Pixmap, ring: &Ring) {
    let inset = ring.stroke / 2.0;
    let Some(rect) = SkRect::from_xywh(
        ring.rect.x + inset,
        ring.rect.y + inset,
        ring.rect.w - ring.stroke,
        ring.rect.h - ring.stroke,
    ) else {
        return;
    };
    let Some(path) = round_rect_path(rect, ring.radius) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(ring.color[0], ring.color[1], ring.color[2], ring.color[3]);
    paint.anti_alias = true;
    let stroke = Stroke {
        width: ring.stroke,
        ..Stroke::default()
    };
    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

fn draw_card(pixmap: &mut Pixmap, card: &Card) {
    let Some(rect) = SkRect::from_xywh(card.rect.x, card.rect.y, card.rect.w, card.rect.h) else {
        return;
    };
    let Some(path) = round_rect_path(rect, card.radius) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(card.bg[0], card.bg[1], card.bg[2], card.bg[3]);
    paint.anti_alias = true;
    pixmap.fill_path(
        &path,
        &paint,
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
    let x = card.rect.x + inset;
    let y = card.rect.bottom() - height - 2.0;
    let Some(rect) = SkRect::from_xywh(x, y, width, height) else {
        return;
    };
    let Some(path) = round_rect_path(rect, height / 2.0) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(
        progress.color[0],
        progress.color[1],
        progress.color[2],
        progress.color[3],
    );
    paint.anti_alias = true;
    pixmap.fill_path(
        &path,
        &paint,
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
    let mut paint = Paint::default();
    paint.set_color_rgba8(
        close.color[0],
        close.color[1],
        close.color[2],
        close.color[3],
    );
    paint.anti_alias = true;
    let stroke = Stroke {
        width: 1.6,
        line_cap: LineCap::Round,
        ..Stroke::default()
    };
    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
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
fn to_premultiplied_bgra(rgba: &[u8]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for px in out.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(buf: &[u8], w: u32, x: u32, y: u32) -> u8 {
        buf[((y * w + x) * 4 + 3) as usize]
    }

    fn ring_frame() -> Frame<'static> {
        Frame {
            width: 100,
            height: 60,
            ring: Some(Ring {
                rect: RectF::new(0.0, 0.0, 100.0, 60.0),
                stroke: 3.0,
                radius: 10.0,
                color: [255, 59, 48, 255],
            }),
            ..Frame::empty(100, 60)
        }
    }

    #[test]
    fn ring_is_hollow() {
        let buf = render_frame(&ring_frame()).unwrap();
        assert_eq!(buf.len(), 100 * 60 * 4);
        assert_eq!(alpha_at(&buf, 100, 50, 30), 0, "中间必须是空的");
        assert!(alpha_at(&buf, 100, 50, 1) > 200, "上边框必须实心");
        assert_eq!(alpha_at(&buf, 100, 0, 0), 0, "圆角外必须透明");
    }

    #[test]
    fn color_channel_order_is_bgra() {
        let spec = Frame {
            width: 32,
            height: 32,
            ring: Some(Ring {
                rect: RectF::new(0.0, 0.0, 32.0, 32.0),
                stroke: 4.0,
                radius: 0.0,
                color: [255, 0, 0, 255],
            }),
            ..Frame::empty(32, 32)
        };
        let buf = render_frame(&spec).unwrap();
        let idx = ((32 + 16) * 4) as usize;
        assert_eq!(buf[idx], 0, "B 通道");
        assert_eq!(buf[idx + 2], 255, "R 通道");
        assert_eq!(buf[idx + 3], 255);
    }

    #[test]
    fn card_is_filled_inside_and_clear_outside() {
        let spec = Frame {
            width: 64,
            height: 64,
            card: Some(Card {
                rect: RectF::new(16.0, 16.0, 32.0, 32.0),
                radius: 6.0,
                bg: [27, 27, 31, 240],
            }),
            ..Frame::empty(64, 64)
        };
        let buf = render_frame(&spec).unwrap();
        assert!(alpha_at(&buf, 64, 32, 32) > 200, "卡片内部应不透明");
        assert_eq!(alpha_at(&buf, 64, 2, 2), 0, "卡片外部应透明");
    }

    #[test]
    fn text_mask_is_blended() {
        let mask = vec![255u8; 4];
        let spec = Frame {
            width: 32,
            height: 32,
            text: Some(TextLayer {
                rect: RectF::new(10.0, 10.0, 2.0, 2.0),
                mask: &mask,
                mask_width: 2,
                mask_height: 2,
                color: [255, 255, 255, 255],
            }),
            ..Frame::empty(32, 32)
        };
        let buf = render_frame(&spec).unwrap();
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
        let full = Frame {
            width: 220,
            height: 80,
            card: Some(card),
            progress: Some(Progress {
                value: 1.0,
                color: [255, 59, 48, 255],
            }),
            ..Frame::empty(220, 80)
        };
        let bar_y = 10 + 60 - 2 - 2 + 1;
        let buf = render_frame(&full).unwrap();
        let idx_full = ((bar_y * 220 + 190) * 4) as usize;
        // 满格时右端是进度条的红色（BGRA 里 R 通道为 255）
        assert_eq!(buf[idx_full + 2], 255, "满格时右端有颜色");

        let empty = Frame {
            progress: Some(Progress {
                value: 0.05,
                color: [255, 59, 48, 255],
            }),
            ..full
        };
        let buf = render_frame(&empty).unwrap();
        // 快结束时右端回到卡片底色，而不是红色
        assert_ne!(buf[idx_full + 2], 255, "快结束时右端不该是进度条颜色");
    }

    #[test]
    fn degenerate_frame_is_rejected() {
        assert!(render_frame(&Frame::empty(0, 10)).is_none());
    }
}
