use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::{PlayMode, Quality};
use crate::paths::Paths;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub api: ApiConfig,
    pub player: PlayerConfig,
    pub ui: UiConfig,
    pub download: DownloadConfig,
    pub local: LocalConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiConfig {
    pub base_url: String,
    pub timeout_secs: u64,
    /// 传给接口的 realIP，绕开部分地区限制；留空不传。
    pub real_ip: String,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:3000".to_string(),
            timeout_secs: 15,
            real_ip: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerConfig {
    pub quality: Quality,
    pub volume: f32,
    pub mode: PlayMode,
    pub cache_limit_mb: u64,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            quality: Quality::ExHigh,
            volume: 0.8,
            mode: PlayMode::ListLoop,
            cache_limit_mb: 2048,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// 启动时恢复上次播放队列。
    pub restore_session: bool,
    /// 启用 MPRIS（桌面媒体控制 / 媒体键）。
    pub mpris: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            restore_session: true,
            mpris: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadConfig {
    /// 下载目录；留空使用 `~/Music/musicyi`。
    pub dir: String,
}

impl DownloadConfig {
    pub fn resolved_dir(&self) -> std::path::PathBuf {
        resolve_dir(&self.dir, "musicyi")
    }
}

/// 本地音乐目录、转换输出与标签补全。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalConfig {
    /// 扫描目录；留空使用 `~/Music/playlist`。
    pub dir: String,
    /// 补全标签时生成的副本目录；留空使用 `~/Music/musicyi/local`。
    pub output_dir: String,
    /// 批量补全时每个请求之间的间隔（毫秒），避免触发网易云风控。
    pub tag_delay_ms: u64,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            dir: String::new(),
            output_dir: String::new(),
            tag_delay_ms: 220,
        }
    }
}

impl LocalConfig {
    pub fn resolved_dir(&self) -> std::path::PathBuf {
        resolve_dir(&self.dir, "playlist")
    }

    pub fn resolved_output_dir(&self) -> std::path::PathBuf {
        resolve_dir(&self.output_dir, "musicyi/local")
    }
}

fn resolve_dir(raw: &str, default_leaf: &str) -> std::path::PathBuf {
    let raw = raw.trim();
    if raw.is_empty() {
        return dirs::audio_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join("Music")))
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(default_leaf);
    }
    if let Some(rest) = raw.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    std::path::PathBuf::from(raw)
}

impl Config {
    pub fn load_or_create(paths: &Paths) -> Result<Self> {
        let path = &paths.config_file;
        if path.exists() {
            let raw = std::fs::read_to_string(path)
                .map_err(|e| Error::Config(format!("读取 {path:?} 失败: {e}")))?;
            let cfg: Config = toml::from_str(&raw)
                .map_err(|e| Error::Config(format!("解析 {path:?} 失败: {e}")))?;
            Ok(cfg)
        } else {
            let cfg = Config::default();
            cfg.save(path)?;
            Ok(cfg)
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        Paths::ensure_parent(path)?;
        let body = toml::to_string_pretty(self)
            .map_err(|e| Error::Config(format!("序列化配置失败: {e}")))?;
        let header = "# musicyi 配置\n# 修改后重启生效。\n\n";
        std::fs::write(path, format!("{header}{body}"))
            .map_err(|e| Error::Config(format!("写入 {path:?} 失败: {e}")))
    }

    pub fn base_url(&self) -> String {
        self.api.base_url.trim_end_matches('/').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn unknown_legacy_source_table_is_ignored() {
        let cfg: Config =
            toml::from_str("[source]\nproxy = \"http://127.0.0.1:7897\"\nunblock = true\n")
                .expect("旧配置里的 [source] 应被忽略");
        assert_eq!(cfg.local.output_dir, "");
    }
}
