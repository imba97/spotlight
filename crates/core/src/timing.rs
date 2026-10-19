//! 停留时长：`--timeout` 的语义与按文案长度的自动估算。
//!
//! 公式见 `design-docs/04-交互与状态机.md` §2：读文字 + 找目标 + 移动鼠标 + 点击，
//! 所以是「固定底 + 随字数增长」，并夹在 6–45 秒之间。

use std::time::Duration;

/// 固定底：认出画面、定位目标所需的时间。
pub const BASE_SECS: f32 = 5.0;
/// 最短停留，再短的文案也不能一闪而过。
pub const MIN_SECS: f32 = 6.0;
/// 最长停留，避免一行长文案把画面钉在屏幕上。
pub const MAX_SECS: f32 = 45.0;
/// 中文阅读速度取 6 字/秒。
pub const CHARS_PER_SEC: f32 = 6.0;

/// 用户/调用方给出的停留意图。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Timeout {
    /// `--timeout 0`：常驻，直到被关闭。
    Sticky,
    /// 显式秒数。
    Seconds(f32),
    /// `--timeout auto` 或未指定。
    #[default]
    Auto,
}

/// 按文案长度估算停留时长。
pub fn estimate_timeout(text: &str) -> Duration {
    let chars = text.chars().filter(|c| !c.is_whitespace()).count() as f32;
    let secs = (BASE_SECS + (chars / CHARS_PER_SEC).ceil()).clamp(MIN_SECS, MAX_SECS);
    Duration::from_secs_f32(secs)
}

/// 解析成真实时长；`None` 表示常驻。
pub fn resolve(timeout: Timeout, text: &str) -> Option<Duration> {
    match timeout {
        Timeout::Sticky => None,
        Timeout::Auto => Some(estimate_timeout(text)),
        Timeout::Seconds(secs) => {
            if secs <= 0.0 {
                None
            } else {
                Some(Duration::from_secs_f32(secs.min(3600.0)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(d: Duration) -> f32 {
        d.as_secs_f32()
    }

    #[test]
    fn empty_text_hits_the_floor() {
        assert_eq!(secs(estimate_timeout("")), MIN_SECS);
        assert_eq!(secs(estimate_timeout("点")), MIN_SECS);
    }

    #[test]
    fn grows_with_length_then_clamps() {
        // 9 字 → 5 + ceil(1.5)=2 → 7
        assert_eq!(secs(estimate_timeout("点这个边框按钮啊")), 7.0);
        // 17 字 → 5 + 3 = 8
        assert_eq!(
            secs(estimate_timeout("先在右侧选择无框线然后再点确定按钮")),
            8.0
        );
        // 很长 → 封顶
        let long = "啊".repeat(600);
        assert_eq!(secs(estimate_timeout(&long)), MAX_SECS);
    }

    #[test]
    fn whitespace_does_not_count() {
        assert_eq!(estimate_timeout("  点 这 里  "), estimate_timeout("点这里"));
    }

    #[test]
    fn sticky_never_expires() {
        assert_eq!(resolve(Timeout::Sticky, "随便"), None);
        assert_eq!(resolve(Timeout::Seconds(0.0), "随便"), None);
        assert_eq!(resolve(Timeout::Seconds(-3.0), "随便"), None);
    }

    #[test]
    fn explicit_seconds_are_capped() {
        assert_eq!(secs(resolve(Timeout::Seconds(12.5), "").unwrap()), 12.5);
        assert_eq!(
            secs(resolve(Timeout::Seconds(99_999.0), "").unwrap()),
            3600.0
        );
    }

    #[test]
    fn default_is_auto() {
        assert_eq!(Timeout::default(), Timeout::Auto);
    }
}
