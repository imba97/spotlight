//! 管道上的 NDJSON 协议。
//!
//! 一行一个 JSON 消息。形状见 `design-docs/01-形态与架构.md` §5：
//!
//! ```jsonc
//! {"v":1,"id":7,"method":"show","params":{...}}
//! {"v":1,"id":7,"ok":true,"result":{...}}
//! ```

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;

/// 稳定的错误码。CLI 会把它映射成退出码，AI 靠它判断下一步。
pub mod codes {
    pub const BAD_PARAMS: &str = "bad_params";
    pub const UNSUPPORTED_VERSION: &str = "unsupported_version";
    pub const BAD_JSON: &str = "bad_json";
    pub const RECT_OUT_OF_BOUNDS: &str = "rect_out_of_bounds";
    pub const NO_SUCH_KEY: &str = "no_such_key";
    pub const INTERNAL: &str = "internal";
    pub const TIMEOUT: &str = "timeout";
    /// 坐标空间没声明（既没 `--capture` 也没 `--space screen`）。
    pub const SPACE_UNSPECIFIED: &str = "space_unspecified";
    /// 引用了不存在的 capture id（可能已过期）。
    pub const UNKNOWN_CAPTURE: &str = "unknown_capture";
    /// 截图失败。
    pub const CAPTURE_FAILED: &str = "capture_failed";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    Show,
    Clear,
    Status,
    Capture,
    Captures,
    Reload,
    Ping,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub v: u32,
    pub id: u64,
    pub method: Method,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl Request {
    pub fn new(id: u64, method: Method, params: Option<Value>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id,
            method,
            params,
        }
    }

    /// 取出并反序列化参数；失败时返回一个带 `hint` 的错误体。
    pub fn parse_params<T: DeserializeOwned>(&self) -> Result<T, ErrorBody> {
        let value = self.params.clone().unwrap_or(Value::Null);
        serde_json::from_value(value).map_err(|e| ErrorBody {
            code: codes::BAD_PARAMS.into(),
            message: format!("参数解析失败: {e}"),
            hint: Some("检查该方法需要的参数结构；`spotlight --help` 里有示例".into()),
        })
    }

    /// 版本兼容性检查。
    pub fn check_version(&self) -> Result<(), ErrorBody> {
        if self.v == PROTOCOL_VERSION {
            Ok(())
        } else {
            Err(ErrorBody {
                code: codes::UNSUPPORTED_VERSION.into(),
                message: format!(
                    "协议版本不匹配：客户端 {}，服务端 {}",
                    self.v, PROTOCOL_VERSION
                ),
                hint: Some("CLI 与守护进程需要来自同一个构建，先 clear 再重试".into()),
            })
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub v: u32,
    pub id: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, error: ErrorBody) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id,
            ok: false,
            result: None,
            error: Some(error),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl ErrorBody {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShowParams {
    /// 单个目标的屏幕物理像素 `[x, y, w, h]`。与 `targets` 至少给一个。
    #[serde(default)]
    pub rect: Option<[i32; 4]>,
    /// 多个目标。`--rect` 重复出现时使用。
    #[serde(default)]
    pub targets: Vec<TargetSpec>,
    /// 坐标是在哪张截图里量的。与 `space` 二选一。
    #[serde(default)]
    pub capture: Option<String>,
    /// `"screen"` 表示坐标已是屏幕物理像素。与 `capture` 二选一。
    #[serde(default)]
    pub space: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub step: Option<String>,
    /// 可选的键盘快捷键提示，例如 Ctrl+B。
    #[serde(default)]
    pub shortcut: Option<String>,
    /// 卡片摆放偏好：auto / top / bottom / left / right。
    #[serde(default)]
    pub placement: Option<String>,
    /// 只校验不显示。show 会返回换算后的屏幕矩形，但不会创建任何窗口。
    #[serde(default)]
    pub dry_run: bool,
    /// 效果链名字；空 = 默认光环。显式给出时**替换**默认组合。
    #[serde(default)]
    pub effects: Vec<String>,
    /// `None` = 自动估算；`Some(0)` = 常驻。
    #[serde(default)]
    pub timeout: Option<f32>,
    #[serde(default)]
    pub key: Option<String>,
    /// 多步流程：给出时按顺序播放，点中当前步骤的高亮区域就前进到下一步。
    #[serde(default)]
    pub steps: Vec<StepSpec>,
}

/// 多步流程里的一步。坐标、超时、效果等沿用外层参数。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StepSpec {
    #[serde(default)]
    pub targets: Vec<TargetSpec>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub shortcut: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TargetSpec {
    pub rect: [i32; 4],
    #[serde(default)]
    pub step: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClearParams {
    #[serde(default)]
    pub all: bool,
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShowResult {
    pub scene: String,
    pub timeout_ms: Option<u64>,
    /// 是否只是一次校验（没有真的显示）。
    #[serde(default)]
    pub dry_run: bool,
    /// 换算后的屏幕物理像素矩形。
    #[serde(default)]
    pub targets: Vec<[i32; 4]>,
    /// 实际生效的效果链。
    #[serde(default)]
    pub effects: Vec<String>,
    /// 渲染时的关键决策，便于调用方自查"框为什么在这儿"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug: Option<ShowDebug>,
    /// 非致命问题，例如文案被截断。调用方应当转告用户。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatusResult {
    pub alive: bool,
    pub version: String,
    pub uptime_ms: u64,
    /// 关闭热键是否注册成功（被占用时为 false）。
    #[serde(default)]
    pub hotkey: bool,
    pub scene: Option<SceneSummary>,
    /// 上一次画面是怎么结束的："completed"（点了完成）或 "stopped"（点了停止）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<String>,
}

/// 守护进程当前持有的截图。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptureSummary {
    pub capture: String,
    pub width: u32,
    pub height: u32,
    /// 距离截图创建过了多久。
    pub age_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapturesResult {
    pub captures: Vec<CaptureSummary>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneSummary {
    pub key: Option<String>,
    pub targets: usize,
    pub has_text: bool,
    pub remaining_ms: Option<u64>,
}

/// 一次渲染的关键决策。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShowDebug {
    /// 卡片落在哪块屏上。
    pub monitor: String,
    /// 该屏的 DPI 缩放。
    pub scale: f32,
    /// 实际采用的方位（小写）。
    pub placement: String,
    /// 卡片在屏幕坐标系下的矩形。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<[i32; 4]>,
    /// 动作按钮在屏幕坐标系下的矩形（调试用）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<[i32; 4]>,
    /// 字号（物理像素，已按 DPI 缩放）。
    pub font_px: i32,
}

/// 重新读取配置的结果。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReloadResult {
    /// 恒为 true，便于 CLI 识别这是重载结果。
    pub reloaded: bool,
    /// 配置文件内容是否有变化。
    pub changed: bool,
    /// 重载后生效的关闭热键。
    pub hotkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CaptureParams {
    /// 长边上限，默认 1280。
    #[serde(default)]
    pub max_width: Option<u32>,
    /// 只截某块屏；缺省 = 主屏。
    #[serde(default)]
    pub monitor: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptureInfo {
    pub capture: String,
    pub image: CaptureImage,
    pub monitors: Vec<MonitorInfo>,
    /// 图与屏的分段对应关系。
    pub segments: Vec<SegmentInfo>,
    pub virtual_screen: RectInfo,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptureImage {
    pub path: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub index: u32,
    pub name: String,
    pub primary: bool,
    /// `[x, y, w, h]`，物理像素。
    pub bounds: [i32; 4],
    pub work: [i32; 4],
    pub scale: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SegmentInfo {
    pub monitor: u32,
    pub image_rect: [i32; 4],
    pub screen_rect: [i32; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RectInfo {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(req: &Request) -> String {
        serde_json::to_string(req).unwrap()
    }

    #[test]
    fn request_round_trips_as_one_line() {
        let req = Request::new(
            7,
            Method::Show,
            Some(serde_json::json!({"rect": [1,2,3,4], "text": "hi"})),
        );
        let s = line(&req);
        assert!(!s.contains('\n'), "必须是一行");
        let back: Request = serde_json::from_str(&s).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn ping_has_no_params() {
        let req = Request::new(1, Method::Ping, None);
        let s = line(&req);
        assert!(!s.contains("params"), "空参数不应出现在线上: {s}");
    }

    #[test]
    fn params_parse_into_show() {
        let req = Request::new(
            2,
            Method::Show,
            Some(serde_json::json!({
                "rect": [10, 20, 30, 40],
                "text": "点这里",
                "timeout": 0
            })),
        );
        let p: ShowParams = req.parse_params().unwrap();
        assert_eq!(p.rect, Some([10, 20, 30, 40]));
        assert_eq!(p.text.as_deref(), Some("点这里"));
        assert_eq!(p.timeout, Some(0.0));
        assert_eq!(p.key, None);
    }

    #[test]
    fn params_parse_error_carries_hint() {
        let req = Request::new(3, Method::Show, Some(serde_json::json!({"rect": "nope"})));
        let err = req.parse_params::<ShowParams>().unwrap_err();
        assert_eq!(err.code, codes::BAD_PARAMS);
        assert!(err.hint.is_some());
    }

    #[test]
    fn version_mismatch_is_reported() {
        let mut req = Request::new(4, Method::Ping, None);
        req.v = 99;
        assert_eq!(
            req.check_version().unwrap_err().code,
            codes::UNSUPPORTED_VERSION
        );
        req.v = PROTOCOL_VERSION;
        assert!(req.check_version().is_ok());
    }

    #[test]
    fn unknown_method_fails_to_deserialize() {
        let err =
            serde_json::from_str::<Request>(r#"{"v":1,"id":1,"method":"explode","params":null}"#);
        assert!(err.is_err());
    }

    #[test]
    fn reload_result_round_trips() {
        let result = ReloadResult {
            reloaded: true,
            changed: true,
            hotkey: "Ctrl+Shift+F8".into(),
            note: None,
        };
        let text = serde_json::to_string(&result).unwrap();
        assert!(!text.contains("note"), "空 note 不应出现在线上");
        let back: ReloadResult = serde_json::from_str(&text).unwrap();
        assert_eq!(back, result);
    }
    #[test]
    fn captures_result_round_trips() {
        let result = CapturesResult {
            captures: vec![CaptureSummary {
                capture: "c-1".into(),
                width: 1280,
                height: 720,
                age_ms: 1500,
            }],
        };
        let text = serde_json::to_string(&result).unwrap();
        assert!(text.contains("c-1"));
        let back: CapturesResult = serde_json::from_str(&text).unwrap();
        assert_eq!(back, result);
    }
    #[test]
    fn response_omits_empty_fields() {
        let ok = Response::ok(9, serde_json::json!({"scene":"s-1"}));
        let s = serde_json::to_string(&ok).unwrap();
        assert!(!s.contains("error"));
        let back: Response = serde_json::from_str(&s).unwrap();
        assert!(back.ok);
        assert!(back.error.is_none());
    }

    #[test]
    fn error_response_round_trips() {
        let resp = Response::err(
            10,
            ErrorBody::new(codes::RECT_OUT_OF_BOUNDS, "越界").with_hint("重新截图"),
        );
        let s = serde_json::to_string(&resp).unwrap();
        let back: Response = serde_json::from_str(&s).unwrap();
        assert!(!back.ok);
        assert_eq!(back.error.unwrap().code, codes::RECT_OUT_OF_BOUNDS);
    }
}
