//! M0 技术验证探针。
//!
//! 证明三件纸面上推不出来的事：
//! 1. 分层遮罩窗口能把屏幕压暗，并且洞里露出的内容不受影响；
//! 2. 窗口确实带上了"穿透 / 不抢焦点 / 不进 Alt+Tab / 置顶"的扩展样式；
//! 3. `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` 能让遮罩不进 `BitBlt` 截图。
//!
//! 结果以 JSON 打到 stdout，全部通过时退出码为 0。

use std::time::Duration;

use spotlight_core::geom::Rect;
use spotlight_core::mask::subtract;
use spotlight_platform::win::{
    MaskWindow, capture_rect, enable_dpi_awareness, mean_diff, pump_for, screen_size,
};

const REGION_W: i32 = 400;
const REGION_H: i32 = 300;
const HOLE_W: i32 = 120;
const HOLE_H: i32 = 120;
const ALPHA: u8 = 140;

fn fail(msg: &str) -> i32 {
    println!("{{\"ok\":false,\"error\":\"{msg}\"}}");
    1
}

pub fn run() -> i32 {
    enable_dpi_awareness();
    let (sw, sh) = screen_size();
    if sw < REGION_W || sh < REGION_H {
        return fail("screen_too_small");
    }

    let region = Rect::new((sw - REGION_W) / 2, (sh - REGION_H) / 2, REGION_W, REGION_H);
    let hole = Rect::new(
        region.x + (REGION_W - HOLE_W) / 2,
        region.y + (REGION_H - HOLE_H) / 2,
        HOLE_W,
        HOLE_H,
    );
    let masks = subtract(region, &[hole]);

    // 1. 先拍一张干净的底图
    let baseline = match capture_rect(region.x, region.y, region.w, region.h) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };

    // 2. 盖上遮罩
    let mut windows = Vec::with_capacity(masks.len());
    for r in &masks {
        match MaskWindow::new(*r, ALPHA, false) {
            Ok(w) => windows.push(w),
            Err(e) => return fail(&e),
        }
    }
    pump_for(Duration::from_millis(350));
    let during = match capture_rect(region.x, region.y, region.w, region.h) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };

    // 3. 让遮罩从截图中隐身
    let affinity_ok = windows.iter().all(|w| w.set_excluded(true));
    pump_for(Duration::from_millis(350));
    let after = match capture_rect(region.x, region.y, region.w, region.h) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };

    let styles = windows.first().map(MaskWindow::styles).unwrap_or_default();
    let styles_ok = styles.layered
        && styles.transparent
        && styles.no_activate
        && styles.tool_window
        && styles.topmost;

    let mad_mask = mean_diff(&baseline, &during, region, &masks);
    let mad_hole = mean_diff(&baseline, &during, region, &[hole]);
    let mad_all = mean_diff(&baseline, &after, region, &[region]);

    // 阈值：遮罩区域必须明显变暗；洞必须基本不变；排除之后必须回到原样。
    let mask_visible = mad_mask > 10.0;
    let hole_intact = mad_hole < 3.0;
    let capture_excluded = mad_all < 3.0;

    drop(windows);
    pump_for(Duration::from_millis(120));

    let ok = mask_visible && hole_intact && capture_excluded && affinity_ok && styles_ok;

    println!(
        "{{\"ok\":{ok},\"screen\":[{sw},{sh}],\
         \"region\":[{},{},{},{}],\"hole\":[{},{},{},{}],\"mask_rects\":{},\
         \"styles\":{{\"layered\":{},\"transparent\":{},\"no_activate\":{},\
         \"tool_window\":{},\"topmost\":{}}},\
         \"checks\":{{\"mask_visible\":{mask_visible},\"hole_intact\":{hole_intact},\
         \"capture_excluded\":{capture_excluded},\"affinity_set\":{affinity_ok}}},\
         \"metrics\":{{\"mad_mask\":{mad_mask:.2},\"mad_hole\":{mad_hole:.2},\
         \"mad_all\":{mad_all:.2}}}}}",
        region.x,
        region.y,
        region.w,
        region.h,
        hole.x,
        hole.y,
        hole.w,
        hole.h,
        masks.len(),
        styles.layered,
        styles.transparent,
        styles.no_activate,
        styles.tool_window,
        styles.topmost,
    );

    if ok { 0 } else { 3 }
}
