//! 音源解析：网易云只使用官方直链与 30 秒试听，雀乐 / 街声使用各自 CDN。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use musicyi_core::model::{Quality, Song, SourceKind};
use musicyi_core::{Error, Result};
use tokio::sync::Mutex;

use crate::client::ApiClient;

#[derive(Debug, Clone)]
pub struct ResolvedSource {
    pub url: String,
    pub provider: String,
    pub br: u64,
    pub format: String,
    /// 是否为试听片段（网易官方 30 秒试听）。
    pub preview: bool,
    /// 解析时顺带拿到的准确时长（街声详情）。
    pub duration_ms: Option<u64>,
    /// 解析时顺带拿到的专辑名（街声详情）。
    pub album: Option<String>,
    /// 解析时顺带拿到的封面（街声详情）。
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone)]
struct CachedSource {
    source: ResolvedSource,
    at: Instant,
}

pub struct Resolver {
    api: Arc<ApiClient>,
    /// 解析结果短时缓存，避免反复触发官方风控。
    cache: Mutex<HashMap<String, CachedSource>>,
}

impl Resolver {
    pub fn new(api: Arc<ApiClient>) -> Self {
        Self {
            api,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// 解析播放地址。网易云拿不到官方直链时只回退试听，不再尝试任何解灰音源。
    pub async fn resolve(&self, song: &Song, quality: Quality) -> Result<ResolvedSource> {
        let cache_key = format!("{}:{}", song.key(), quality.level());
        if let Some(source) = self.cached(&cache_key).await {
            return Ok(source);
        }

        match song.source {
            SourceKind::Indie => {
                let url = song
                    .play_url
                    .clone()
                    .filter(|u| !u.trim().is_empty())
                    .ok_or_else(|| Error::Api("雀乐歌曲缺少播放地址".into()))?;
                let source = ResolvedSource {
                    url,
                    provider: "雀乐".into(),
                    br: 0,
                    format: "mp3".into(),
                    preview: false,
                    duration_ms: None,
                    album: None,
                    cover_url: None,
                };
                self.remember(&cache_key, &source).await;
                return Ok(source);
            }
            SourceKind::StreetVoice => {
                let detail = self.api.streetvoice_detail(&song.id).await.ok();
                let source = ResolvedSource {
                    url: format!(
                        "{}/musicyi/stream/streetvoice/{}",
                        self.api.base_url(),
                        song.id
                    ),
                    provider: "街声".into(),
                    br: 96_000,
                    format: "mp3".into(),
                    preview: false,
                    duration_ms: detail.as_ref().map(|d| d.duration_ms).filter(|d| *d > 0),
                    album: detail
                        .as_ref()
                        .map(|d| d.album.clone())
                        .filter(|a| !a.trim().is_empty()),
                    cover_url: detail.as_ref().and_then(|d| d.cover_url.clone()),
                };
                self.remember(&cache_key, &source).await;
                return Ok(source);
            }
            SourceKind::Local => {
                return Err(Error::Api("本地文件不需要解析音源".into()));
            }
            _ => {}
        }

        let mut errors = Vec::new();
        let mut trial = None;

        // 官方直链：从请求档位逐级降档，避免无损不可用导致整首失败。
        let mut level = Some(quality);
        while let Some(q) = level {
            match self.api.song_url(&song.id, q).await {
                Ok(Some(u)) => {
                    let source = ResolvedSource {
                        url: u.url,
                        provider: if u.trial {
                            "网易云·试听".into()
                        } else {
                            "网易云".into()
                        },
                        br: u.br,
                        format: if u.format.is_empty() {
                            "mp3".into()
                        } else {
                            u.format
                        },
                        preview: u.trial,
                        duration_ms: None,
                        album: None,
                        cover_url: None,
                    };
                    if !u.trial {
                        self.remember(&cache_key, &source).await;
                        return Ok(source);
                    }
                    if trial.is_none() {
                        trial = Some(source);
                    }
                }
                Ok(None) => errors.push(format!("官方直链 {} 为空", q.level())),
                Err(e) => errors.push(format!("官方直链 {} 失败: {e}", q.level())),
            }
            level = q.downgrade();
        }

        if let Some(trial) = trial {
            return Ok(trial);
        }

        Err(Error::Api(format!(
            "网易云官方音源不可用（{}）",
            errors.join("；")
        )))
    }

    async fn cached(&self, key: &str) -> Option<ResolvedSource> {
        let mut cache = self.cache.lock().await;
        match cache.get(key) {
            Some(entry) if entry.at.elapsed() < Duration::from_secs(300) => {
                Some(entry.source.clone())
            }
            Some(_) => {
                cache.remove(key);
                None
            }
            None => None,
        }
    }

    async fn remember(&self, key: &str, source: &ResolvedSource) {
        let mut cache = self.cache.lock().await;
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(
            key.to_string(),
            CachedSource {
                source: source.clone(),
                at: Instant::now(),
            },
        );
    }
}
