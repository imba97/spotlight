//! 抓屏、等比缩小、存成 PNG。

use std::path::Path;

use spotlight_core::geom::Rect;

use super::capture_rect;

pub struct Shot {
    pub width: i32,
    pub height: i32,
    pub bgra: Vec<u8>,
}

pub fn grab(rect: Rect) -> Result<Shot, String> {
    let captured = capture_rect(rect.x, rect.y, rect.w, rect.h)?;
    Ok(Shot {
        width: captured.width,
        height: captured.height,
        bgra: captured.bgra,
    })
}

/// 按长边上限等比缩小；不需要缩小时返回 `None`。
pub fn downscale(shot: &Shot, max_width: u32) -> Option<Shot> {
    let max_width = max_width.max(16) as i32;
    if shot.width <= max_width {
        return None;
    }
    let dw = max_width;
    let dh = ((shot.height as i64 * max_width as i64) / shot.width as i64).max(1) as i32;
    let bgra = box_downscale(&shot.bgra, shot.width, shot.height, dw, dh);
    Some(Shot {
        width: dw,
        height: dh,
        bgra,
    })
}

/// 面积平均缩小，比最近邻更适合缩文字。
fn box_downscale(src: &[u8], sw: i32, sh: i32, dw: i32, dh: i32) -> Vec<u8> {
    let mut out = vec![0u8; (dw as usize) * (dh as usize) * 4];
    for dy in 0..dh {
        let y0 = dy * sh / dh;
        let y1 = ((dy + 1) * sh / dh).max(y0 + 1).min(sh);
        for dx in 0..dw {
            let x0 = dx * sw / dw;
            let x1 = ((dx + 1) * sw / dw).max(x0 + 1).min(sw);
            let mut acc = [0u32; 4];
            let mut count = 0u32;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = ((sy as usize * sw as usize) + sx as usize) * 4;
                    for c in 0..4 {
                        acc[c] += u32::from(src[i + c]);
                    }
                    count += 1;
                }
            }
            let o = ((dy as usize * dw as usize) + dx as usize) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / count.max(1)) as u8;
            }
        }
    }
    out
}

/// 写 PNG（BGRA 输入，落盘为 RGBA）。
pub fn save_png(path: &Path, width: u32, height: u32, bgra: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let file = std::fs::File::create(path).map_err(|e| format!("创建文件失败: {e}"))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;

    let mut rgba = Vec::with_capacity(bgra.len());
    for px in bgra.chunks_exact(4) {
        rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }
    writer
        .write_image_data(&rgba)
        .map_err(|e| format!("写 PNG 失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: i32, h: i32, color: [u8; 4]) -> Shot {
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h {
            bgra.extend_from_slice(&color);
        }
        Shot {
            width: w,
            height: h,
            bgra,
        }
    }

    #[test]
    fn downscale_preserves_aspect_ratio() {
        let shot = solid(1920, 1080, [10, 20, 30, 255]);
        let small = downscale(&shot, 1280).unwrap();
        assert_eq!(small.width, 1280);
        assert_eq!(small.height, 720);
        assert_eq!(small.bgra.len(), 1280 * 720 * 4);
    }

    #[test]
    fn small_images_are_not_scaled() {
        let shot = solid(800, 600, [1, 2, 3, 255]);
        assert!(downscale(&shot, 1280).is_none());
    }

    #[test]
    fn downscale_keeps_color() {
        let shot = solid(200, 100, [10, 20, 30, 255]);
        let small = downscale(&shot, 100).unwrap();
        assert_eq!(&small.bgra[0..4], &[10, 20, 30, 255]);
    }
}
