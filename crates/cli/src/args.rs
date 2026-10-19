//! 极简参数解析。
//!
//! 不引入 clap：这不是省 300KB 的问题，而是这个 CLI 的参数面就十来个，
//! 手写更容易控制"缺参数时报什么错"——而错误信息是给模型看的。
//!
//! 同一个开关可以重复出现（`--rect a --rect b`），重复值收在 `multi` 里。

use std::collections::{HashMap, HashSet};

pub struct Parsed {
    pub values: HashMap<String, String>,
    pub multi: HashMap<String, Vec<String>>,
    pub switches: HashSet<String>,
}

impl Parsed {
    pub fn value(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// 某个开关出现过的所有值，按出现顺序。
    pub fn all(&self, key: &str) -> &[String] {
        self.multi.get(key).map_or(&[], Vec::as_slice)
    }

    pub fn flag(&self, key: &str) -> bool {
        self.switches.contains(key)
    }
}

pub fn parse(tokens: &[String]) -> Result<Parsed, String> {
    let mut values = HashMap::new();
    let mut multi: HashMap<String, Vec<String>> = HashMap::new();
    let mut switches = HashSet::new();
    let mut it = tokens.iter().peekable();

    while let Some(token) = it.next() {
        let Some(name) = token.strip_prefix("--") else {
            return Err(format!("无法识别的参数: {token}"));
        };
        if name.is_empty() {
            return Err("空的参数名".into());
        }

        let (key, inline) = match name.split_once('=') {
            Some((key, value)) => (key.to_string(), Some(value.to_string())),
            None => (name.to_string(), None),
        };

        let value = match inline {
            Some(value) => Some(value),
            None => match it.peek() {
                Some(next) if !next.starts_with("--") => {
                    let value = (*next).clone();
                    it.next();
                    Some(value)
                }
                _ => None,
            },
        };

        match value {
            Some(value) => {
                multi.entry(key.clone()).or_default().push(value.clone());
                values.insert(key, value);
            }
            None => {
                switches.insert(key);
            }
        }
    }

    Ok(Parsed {
        values,
        multi,
        switches,
    })
}

/// 解析 `"x,y,w,h"`。
pub fn parse_rect(raw: &str) -> Result<[i32; 4], String> {
    let parts: Vec<&str> = raw.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return Err(format!("--rect 需要四个数字 x,y,w,h，收到: {raw}"));
    }
    let mut out = [0i32; 4];
    for (i, part) in parts.iter().enumerate() {
        out[i] = part
            .parse::<i32>()
            .map_err(|_| format!("--rect 第 {} 个值不是整数: {part}", i + 1))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn reads_values_and_switches() {
        let p = parse(&toks("--rect 1,2,3,4 --json")).unwrap();
        assert_eq!(p.value("rect"), Some("1,2,3,4"));
        assert!(p.flag("json"));
        assert!(!p.flag("rect"));
    }

    #[test]
    fn supports_inline_equals() {
        let p = parse(&toks("--rect=10,20,30,40")).unwrap();
        assert_eq!(p.value("rect"), Some("10,20,30,40"));
    }

    #[test]
    fn switch_before_another_flag_is_not_eaten() {
        let p = parse(&toks("--json --rect 1,2,3,4")).unwrap();
        assert!(p.flag("json"));
        assert_eq!(p.value("rect"), Some("1,2,3,4"));
    }

    #[test]
    fn repeated_flags_are_collected() {
        let p = parse(&toks("--rect 1,2,3,4 --rect 5,6,7,8 --effect brush")).unwrap();
        assert_eq!(p.all("rect").len(), 2);
        assert_eq!(p.all("rect")[1], "5,6,7,8");
        assert_eq!(p.all("effect"), ["brush".to_string()]);
        // 最后一次的值仍然可以从 value 取到
        assert_eq!(p.value("rect"), Some("5,6,7,8"));
    }

    #[test]
    fn rejects_positional_args() {
        assert!(parse(&toks("show")).is_err());
    }

    #[test]
    fn rect_parsing() {
        assert_eq!(parse_rect("-10,20,30,40").unwrap(), [-10, 20, 30, 40]);
        assert!(parse_rect("1,2,3").is_err());
        assert!(parse_rect("1,2,x,4").is_err());
    }
}
