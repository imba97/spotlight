//! 全局热键：解析 `Ctrl+Alt+Esc` 这类字符串，并注册到当前线程。

use windows_sys::Win32::UI::Input::KeyboardAndMouse::RegisterHotKey;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey;

/// 修饰键位（与 Win32 的 MOD_* 一致）。
const MOD_ALT: u32 = 0x0001;
const MOD_CONTROL: u32 = 0x0002;
const MOD_SHIFT: u32 = 0x0004;
const MOD_WIN: u32 = 0x0008;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: u32,
    pub vk: u32,
}

pub fn default_dismiss() -> Hotkey {
    Hotkey {
        modifiers: MOD_CONTROL | MOD_ALT,
        vk: 0x1B, // Esc
    }
}

/// 解析 `Ctrl+Alt+Esc`。至少需要一个修饰键——单独一个按键会被全局截获，太危险。
pub fn parse(text: &str) -> Result<Hotkey, String> {
    let parts: Vec<&str> = text
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() < 2 {
        return Err(format!("热键至少要有一个修饰键和一个按键: {text}"));
    }

    let (modifier_parts, key_part) = parts.split_at(parts.len() - 1);
    let mut modifiers = 0u32;
    for part in modifier_parts {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= MOD_CONTROL,
            "alt" => modifiers |= MOD_ALT,
            "shift" => modifiers |= MOD_SHIFT,
            "win" | "meta" | "super" => modifiers |= MOD_WIN,
            other => return Err(format!("不认识的修饰键: {other}")),
        }
    }

    let vk = virtual_key(key_part[0]).ok_or_else(|| format!("不认识的按键: {}", key_part[0]))?;
    Ok(Hotkey { modifiers, vk })
}

fn virtual_key(key: &str) -> Option<u32> {
    let lower = key.to_ascii_lowercase();

    // 单字符：字母和数字
    let mut chars = lower.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_alphabetic() {
            return Some(0x41 + (c as u32 - 'a' as u32));
        }
        if c.is_ascii_digit() {
            return Some(0x30 + (c as u32 - '0' as u32));
        }
    }

    // 功能键 F1–F24
    if let Some(rest) = lower.strip_prefix('f') {
        if let Ok(number) = rest.parse::<u32>() {
            if (1..=24).contains(&number) {
                return Some(0x70 + number - 1);
            }
        }
    }

    Some(match lower.as_str() {
        "esc" | "escape" => 0x1B,
        "space" => 0x20,
        "tab" => 0x09,
        "enter" | "return" => 0x0D,
        "backspace" => 0x08,
        "delete" | "del" => 0x2E,
        "insert" | "ins" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdn" => 0x22,
        "up" => 0x26,
        "down" => 0x28,
        "left" => 0x25,
        "right" => 0x27,
        _ => return None,
    })
}

/// 注册（覆盖之前的同名 id）。返回是否成功——被别的程序占用时会失败。
pub fn register(id: i32, hotkey: Hotkey) -> bool {
    unsafe { RegisterHotKey(std::ptr::null_mut(), id, hotkey.modifiers, hotkey.vk) != 0 }
}

pub fn unregister(id: i32) {
    unsafe {
        UnregisterHotKey(std::ptr::null_mut(), id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_ctrl_alt_esc() {
        let spec = default_dismiss();
        assert_eq!(spec.modifiers, MOD_CONTROL | MOD_ALT);
        assert_eq!(spec.vk, 0x1B);
    }

    #[test]
    fn parses_common_combinations() {
        assert_eq!(parse("Ctrl+Alt+Esc").unwrap(), default_dismiss());
        assert_eq!(
            parse("ctrl + shift + f5").unwrap(),
            Hotkey {
                modifiers: MOD_CONTROL | MOD_SHIFT,
                vk: 0x74
            }
        );
        assert_eq!(
            parse("Win+Shift+A").unwrap(),
            Hotkey {
                modifiers: MOD_WIN | MOD_SHIFT,
                vk: 0x41
            }
        );
        assert_eq!(
            parse("Alt+Space").unwrap(),
            Hotkey {
                modifiers: MOD_ALT,
                vk: 0x20
            }
        );
    }

    #[test]
    fn rejects_bare_key() {
        // 单键会被全局截获，必须拒绝
        assert!(parse("Esc").is_err());
        assert!(parse("F5").is_err());
    }

    #[test]
    fn rejects_unknown_parts() {
        assert!(parse("Ctrl+Bogus+Esc").is_err());
        assert!(parse("Ctrl+Alt+Nope").is_err());
        assert!(parse("+").is_err());
    }
}
