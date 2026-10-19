//! 一次调用的完整画面。

use crate::geom::ScreenRect;
use crate::timing::Timeout;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    /// `--id`：相同 key 的 show 会原地替换，不重播入场动画。
    pub key: Option<String>,
    pub targets: Vec<Target>,
    pub callout: Option<Callout>,
    pub timeout: Timeout,
}

impl Scene {
    pub fn new(target: ScreenRect) -> Self {
        Self {
            key: None,
            targets: vec![Target::new(target)],
            callout: None,
            timeout: Timeout::Auto,
        }
    }

    pub fn validate(&self) -> Result<(), SceneError> {
        if self.targets.is_empty() {
            return Err(SceneError::NoTargets);
        }
        if self.targets.iter().any(|t| t.rect.rect().is_empty()) {
            return Err(SceneError::EmptyTarget);
        }
        if self.callout.as_ref().is_some_and(Callout::is_empty) {
            return Err(SceneError::EmptyCallout);
        }
        Ok(())
    }

    pub fn text(&self) -> &str {
        self.callout.as_ref().map_or("", |c| c.text.as_str())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub rect: ScreenRect,
    pub step: Option<String>,
}

impl Target {
    pub fn new(rect: ScreenRect) -> Self {
        Self { rect, step: None }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Callout {
    pub title: Option<String>,
    pub text: String,
}

impl Callout {
    pub fn is_empty(&self) -> bool {
        // 只有标题没有正文也是合法卡片。
        self.title.is_none() && self.text.trim().is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneError {
    NoTargets,
    EmptyTarget,
    EmptyCallout,
}

impl SceneError {
    /// 给调用方的下一步建议。错误信息本身只描述问题，hint 负责告诉它怎么办。
    pub fn hint(&self) -> &'static str {
        match self {
            SceneError::NoTargets => "至少给一个 --rect，或者用 --file 传 targets 数组",
            SceneError::EmptyTarget => "检查截图里量出的矩形，w 和 h 都必须大于 0",
            SceneError::EmptyCallout => "给 --text 写点内容，或者不要传 --title",
        }
    }
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SceneError::NoTargets => "场景没有任何目标",
            SceneError::EmptyTarget => "目标矩形为空",
            SceneError::EmptyCallout => "文案为空",
        };
        f.write_str(s)
    }
}

impl std::error::Error for SceneError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;

    #[test]
    fn empty_scene_is_invalid() {
        let scene = Scene::default();
        assert_eq!(scene.validate(), Err(SceneError::NoTargets));
    }

    #[test]
    fn zero_sized_target_is_invalid() {
        let mut scene = Scene::new(ScreenRect::new(10, 10, 0, 20));
        assert_eq!(scene.validate(), Err(SceneError::EmptyTarget));
        scene.targets[0].rect = ScreenRect::new(10, 10, 20, 20);
        assert_eq!(scene.validate(), Ok(()));
    }

    #[test]
    fn whitespace_only_callout_is_invalid() {
        let mut scene = Scene::new(ScreenRect::new(0, 0, 10, 10));
        scene.callout = Some(Callout {
            title: None,
            text: "   ".into(),
        });
        assert_eq!(scene.validate(), Err(SceneError::EmptyCallout));
        // 完全空的 callout 是合法的（只画框不说话）
        scene.callout = None;
        assert_eq!(scene.validate(), Ok(()));
    }

    #[test]
    fn every_error_carries_a_hint() {
        for error in [
            SceneError::NoTargets,
            SceneError::EmptyTarget,
            SceneError::EmptyCallout,
        ] {
            assert!(!error.hint().is_empty(), "{error:?} 缺少 hint");
        }
    }
    #[test]
    fn text_is_empty_without_callout() {
        assert_eq!(Scene::default().text(), "");
        let mut s = Scene::new(ScreenRect::from_rect(Rect::new(0, 0, 4, 4)));
        s.callout = Some(Callout {
            title: None,
            text: "点这里".into(),
        });
        assert_eq!(s.text(), "点这里");
    }
}
