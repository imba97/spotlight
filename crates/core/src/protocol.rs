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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    Show,
    Clear,
    Status,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShowParams {
    /// 屏幕物理像素：`[x, y, w, h]`。
    pub rect: [i32; 4],
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub step: Option<String>,
    /// `None` = 自动估算；`Some(0)` = 常驻。
    #[serde(default)]
    pub timeout: Option<f32>,
    #[serde(default)]
    pub key: Option<String>,
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatusResult {
    pub alive: bool,
    pub version: String,
    pub uptime_ms: u64,
    pub scene: Option<SceneSummary>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneSummary {
    pub key: Option<String>,
    pub targets: usize,
    pub has_text: bool,
    pub remaining_ms: Option<u64>,
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
        assert_eq!(p.rect, [10, 20, 30, 40]);
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
