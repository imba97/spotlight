//! 数据目录与配置文件位置。
//!
//! 默认在 `%LOCALAPPDATA%\spotlight`；设置 `SPOTLIGHT_HOME` 可以整体挪走，
//! 便携部署和自动化测试都靠它。

use std::path::PathBuf;

pub fn data_home() -> PathBuf {
    if let Some(home) = std::env::var_os("SPOTLIGHT_HOME") {
        return PathBuf::from(home);
    }
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("spotlight")
}

pub fn config_path() -> PathBuf {
    data_home().join("config.toml")
}

pub fn captures_dir() -> PathBuf {
    data_home().join("captures")
}
