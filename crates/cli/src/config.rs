//! 用户配置：`<数据目录>/config.toml`。
//!
//! 设计见 `design-docs/03-渲染与视觉.md` §7。原则：
//! - 文件不存在 = 全用默认值，不报错
//! - 某个字段写错 = 该项回退默认值并留下一条说明，其余配置照常生效
//! - 数值越界 = 夹到安全区间，而不是拒绝启动

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// RGBA 强调色，光环 / 圈画 / 进度条都用它。
    pub accent: [u8; 4],
    /// 遮罩不透明度，0.05–0.9。
    pub dim: f32,
    pub radius: f32,
    pub ring_width: f32,
    pub font_px: i32,
    pub max_text_width: u32,
    /// 悬停暂停的累计上限。
    pub pause_cap_secs: u64,
    /// 没显式指定 `--effect` 时用哪组效果。
    pub default_effects: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            accent: [255, 59, 48, 255],
            dim: 0.55,
            radius: 10.0,
            ring_width: 3.0,
            font_px: 14,
            max_text_width: 336,
            pause_cap_secs: 60,
            default_effects: vec!["ring".to_string()],
        }
    }
}

/// 读取结果。`note` 用于告诉用户"配置有问题，用了默认值"，不阻断启动。
pub struct Loaded {
    pub config: Config,
    pub path: PathBuf,
    pub exists: bool,
    pub note: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Raw {
    #[serde(default)]
    visual: RawVisual,
    #[serde(default)]
    callout: RawCallout,
    #[serde(default)]
    effects: RawEffects,
    #[serde(default)]
    pause: RawPause,
}

#[derive(Debug, Default, Deserialize)]
struct RawVisual {
    #[serde(default)]
    accent: Option<String>,
    #[serde(default)]
    dim: Option<f32>,
    #[serde(default)]
    radius: Option<f32>,
    #[serde(default)]
    ring_width: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
struct RawCallout {
    #[serde(default)]
    font_px: Option<i32>,
    #[serde(default)]
    max_text_width: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct RawEffects {
    #[serde(default)]
    default: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawPause {
    #[serde(default)]
    cap_seconds: Option<u64>,
}

pub fn load(path: &Path) -> Loaded {
    let mut config = Config::default();

    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Loaded {
                config,
                path: path.to_path_buf(),
                exists: false,
                note: None,
            };
        }
        Err(err) => {
            return Loaded {
                config,
                path: path.to_path_buf(),
                exists: true,
                note: Some(format!("读取失败: {err}")),
            };
        }
    };

    let raw: Raw = match toml::from_str(&text) {
        Ok(raw) => raw,
        Err(err) => {
            return Loaded {
                config,
                path: path.to_path_buf(),
                exists: true,
                note: Some(format!("解析失败: {err}")),
            };
        }
    };

    let mut warnings = Vec::new();
    apply(&mut config, &raw, &mut warnings);

    Loaded {
        config,
        path: path.to_path_buf(),
        exists: true,
        note: (!warnings.is_empty()).then(|| warnings.join("；")),
    }
}

fn apply(config: &mut Config, raw: &Raw, warnings: &mut Vec<String>) {
    if let Some(text) = &raw.visual.accent {
        match parse_color(text) {
            Some(color) => config.accent = color,
            None => warnings.push(format!("visual.accent 不是合法颜色: {text}")),
        }
    }
    if let Some(dim) = raw.visual.dim {
        config.dim = dim.clamp(0.05, 0.9);
    }
    if let Some(radius) = raw.visual.radius {
        config.radius = radius.clamp(0.0, 40.0);
    }
    if let Some(width) = raw.visual.ring_width {
        config.ring_width = width.clamp(1.0, 10.0);
    }
    if let Some(font) = raw.callout.font_px {
        config.font_px = font.clamp(8, 48);
    }
    if let Some(width) = raw.callout.max_text_width {
        config.max_text_width = width.clamp(80, 800);
    }
    if let Some(secs) = raw.pause.cap_seconds {
        config.pause_cap_secs = secs.min(600);
    }
    if !raw.effects.default.is_empty() {
        config.default_effects = raw.effects.default.clone();
    }
}

/// 支持 `#RRGGBB` 和 `#RRGGBBAA`，`#` 可省略。
pub fn parse_color(text: &str) -> Option<[u8; 4]> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }
    let mut out = [0u8, 0, 0, 255];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let part = std::str::from_utf8(chunk).ok()?;
        out[i] = u8::from_str_radix(part, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试文件放在工作区的 target/ 下：CI 和受限沙箱里都保证可写。
    fn write_temp(name: &str, body: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("test-config");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.toml"));
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn missing_file_uses_defaults() {
        let loaded = load(Path::new("D:/definitely/not/here/config.toml"));
        assert!(!loaded.exists);
        assert!(loaded.note.is_none());
        assert_eq!(loaded.config, Config::default());
    }

    #[test]
    fn full_config_is_applied() {
        let path = write_temp(
            "full",
            r##"
[visual]
accent = "#00C853"
dim = 0.7
radius = 14
ring_width = 5

[callout]
font_px = 18
max_text_width = 400

[effects]
default = ["brush"]

[pause]
cap_seconds = 30
"##,
        );
        let loaded = load(&path);
        let cfg = loaded.config;
        assert_eq!(cfg.accent, [0, 200, 83, 255]);
        assert_eq!(cfg.dim, 0.7);
        assert_eq!(cfg.radius, 14.0);
        assert_eq!(cfg.ring_width, 5.0);
        assert_eq!(cfg.font_px, 18);
        assert_eq!(cfg.max_text_width, 400);
        assert_eq!(cfg.pause_cap_secs, 30);
        assert_eq!(cfg.default_effects, vec!["brush".to_string()]);
        assert!(loaded.note.is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn invalid_color_keeps_default_and_warns() {
        let path = write_temp("badcolor", "[visual]\naccent = \"nope\"\n");
        let loaded = load(&path);
        assert_eq!(loaded.config.accent, Config::default().accent);
        assert!(loaded.note.unwrap().contains("accent"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let path = write_temp(
            "clamp",
            "[visual]\ndim = 5.0\nradius = 999.0\nring_width = 0.1\n\n[callout]\nfont_px = 2\n",
        );
        let cfg = load(&path).config;
        assert_eq!(cfg.dim, 0.9);
        assert_eq!(cfg.radius, 40.0);
        assert_eq!(cfg.ring_width, 1.0);
        assert_eq!(cfg.font_px, 8);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn broken_toml_falls_back_to_defaults() {
        let path = write_temp("broken", "[visual\naccent = ");
        let loaded = load(&path);
        assert_eq!(loaded.config, Config::default());
        assert!(loaded.note.unwrap().contains("解析失败"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn color_parsing() {
        assert_eq!(parse_color("#FF3B30"), Some([255, 59, 48, 255]));
        assert_eq!(parse_color("ff3b30"), Some([255, 59, 48, 255]));
        assert_eq!(parse_color("#FF3B3080"), Some([255, 59, 48, 128]));
        assert_eq!(parse_color("#XYZ"), None);
        assert_eq!(parse_color("#GG0000"), None);
    }
}
