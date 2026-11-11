use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// 全部可写路径的集中约定。
///
/// 默认遵循 XDG：
/// - 配置 `~/.config/musicyi/config.toml`
/// - 数据 `~/.local/share/musicyi/`
/// - 缓存 `~/.cache/musicyi/`
/// - 日志 `~/.local/state/musicyi/logs/`
///
/// 设置 `MUSICYI_ROOT` 后全部收拢到该目录下，便于做便携包与测试。
#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub config_file: PathBuf,
    pub data_dir: PathBuf,
    pub history_file: PathBuf,
    pub session_file: PathBuf,
    pub playlists_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub audio_cache: PathBuf,
    pub log_dir: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        if let Some(root) = std::env::var_os("MUSICYI_ROOT") {
            let root = PathBuf::from(root);
            let data_dir = root.join("data");
            return Ok(Self {
                config_file: root.join("config.toml"),
                history_file: data_dir.join("history.jsonl"),
                session_file: data_dir.join("session.json"),
                playlists_dir: data_dir.join("playlists"),
                cache_dir: root.join("cache"),
                audio_cache: root.join("cache/audio"),
                log_dir: root.join("logs"),
                data_dir,
                root,
            });
        }

        let config_dir = dirs::config_dir()
            .ok_or_else(|| Error::Path("找不到用户配置目录".into()))?
            .join("musicyi");
        let data_dir = dirs::data_dir()
            .ok_or_else(|| Error::Path("找不到用户数据目录".into()))?
            .join("musicyi");
        let cache_dir = dirs::cache_dir()
            .ok_or_else(|| Error::Path("找不到用户缓存目录".into()))?
            .join("musicyi");
        let log_dir = dirs::state_dir()
            .or_else(dirs::data_dir)
            .ok_or_else(|| Error::Path("找不到用户状态目录".into()))?
            .join("musicyi/logs");

        Ok(Self {
            root: data_dir.clone(),
            config_file: config_dir.join("config.toml"),
            history_file: data_dir.join("history.jsonl"),
            session_file: data_dir.join("session.json"),
            playlists_dir: data_dir.join("playlists"),
            audio_cache: cache_dir.join("audio"),
            cache_dir,
            log_dir,
            data_dir,
        })
    }

    pub fn ensure(&self) -> Result<()> {
        for dir in [
            self.config_file.parent(),
            Some(self.data_dir.as_path()),
            Some(self.playlists_dir.as_path()),
            Some(self.cache_dir.as_path()),
            Some(self.audio_cache.as_path()),
            Some(self.log_dir.as_path()),
        ]
        .into_iter()
        .flatten()
        {
            std::fs::create_dir_all(dir)
                .map_err(|e| Error::Path(format!("创建 {dir:?} 失败: {e}")))?;
        }
        Ok(())
    }

    pub fn as_display(&self) -> String {
        self.root.display().to_string()
    }

    pub fn ensure_parent(path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Path(format!("创建 {parent:?} 失败: {e}")))?;
        }
        Ok(())
    }
}
