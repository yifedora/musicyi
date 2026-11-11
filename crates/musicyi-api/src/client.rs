use std::time::Duration;

use musicyi_core::config::ApiConfig;
use musicyi_core::model::{Playlist, Quality, Song, SourceKind};
use musicyi_core::{Error, Result};
use serde_json::Value;

use crate::dto;

#[derive(Debug, Clone, Default)]
pub struct SongUrl {
    pub url: String,
    pub br: u64,
    pub size: u64,
    pub format: String,
    /// 官方只返回 30 秒试听（`freeTrialInfo` 非空）。
    pub trial: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RawLyric {
    pub lrc: String,
    pub tlyric: String,
}

/// 街声单曲详情（播放前用于补时长、专辑与歌词）。
#[derive(Debug, Clone, Default)]
pub struct StreetVoiceDetail {
    pub duration_ms: u64,
    pub album: String,
    pub cover_url: Option<String>,
    pub lyrics: String,
    pub lyrics_is_lrc: bool,
}

/// 对 netease-cloud-music-api 的薄封装。
pub struct ApiClient {
    http: reqwest::Client,
    base: String,
    real_ip: Option<String>,
}

impl ApiClient {
    pub fn new(cfg: &ApiConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_secs.clamp(3, 60)))
            .user_agent(
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/131.0 Safari/537.36 musicyi/0.1",
            )
            .build()
            .map_err(|e| Error::Network(format!("构建 HTTP 客户端失败: {e}")))?;
        let real_ip = if cfg.real_ip.trim().is_empty() {
            None
        } else {
            Some(cfg.real_ip.trim().to_string())
        };
        Ok(Self {
            http,
            base: cfg.base_url.trim_end_matches('/').to_string(),
            real_ip,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// 启动时的连通性检查。
    pub async fn ping(&self) -> Result<()> {
        let v = self
            .get("search/hot/detail", vec![])
            .await
            .map_err(|e| Error::Network(format!("无法连接 {}: {e}", self.base)))?;
        if v.get("code").and_then(Value::as_i64).unwrap_or(200) == 200 {
            Ok(())
        } else {
            Err(Error::Api(format!("{} 返回异常", self.base)))
        }
    }

    async fn get(&self, path: &str, params: Vec<(&str, String)>) -> Result<Value> {
        self.get_inner(path, params, false).await
    }

    /// 本机 musicyi 路由：绕开 apicache，避免旧目录/同名歌曲串数据。
    async fn get_local(&self, path: &str, params: Vec<(&str, String)>) -> Result<Value> {
        self.get_inner(path, params, true).await
    }

    async fn get_inner(
        &self,
        path: &str,
        params: Vec<(&str, String)>,
        bypass_cache: bool,
    ) -> Result<Value> {
        let url = format!("{}/{}", self.base, path.trim_start_matches('/'));
        let mut last_err = String::new();
        // 上游（网易）偶发 ECONNRESET 与 -460 风控，多试几次通常就能过。
        for attempt in 0..4 {
            let mut req = self.http.get(&url).query(&params);
            if bypass_cache {
                req = req.header("x-apicache-bypass", "1");
            }
            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp
                        .text()
                        .await
                        .map_err(|e| Error::Network(format!("读取响应失败: {e}")))?;
                    if !status.is_success() {
                        last_err = format!(
                            "HTTP {status}: {}",
                            text.chars().take(160).collect::<String>()
                        );
                    } else {
                        let value: Value = serde_json::from_str(&text).map_err(|e| {
                            Error::Api(format!(
                                "响应不是合法 JSON: {e}: {}",
                                text.chars().take(120).collect::<String>()
                            ))
                        })?;
                        match value.get("code").and_then(Value::as_i64) {
                            Some(301) => {
                                return Err(Error::Api("需要登录后才能使用该功能".into()));
                            }
                            Some(code) if !(0..500).contains(&code) => {
                                // -460 是风控，502 是上游连接被重置：都不立即失败。
                                let msg = value
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("接口返回错误");
                                last_err = format!("{msg} (code {code})");
                            }
                            _ => return Ok(value),
                        }
                    }
                }
                Err(e) => last_err = e.to_string(),
            }
            if attempt < 3 {
                tokio::time::sleep(Duration::from_millis(400 * (attempt as u64 + 1))).await;
            }
        }
        Err(Error::Network(format!("请求 {url} 失败: {last_err}")))
    }

    pub async fn search_songs(&self, keywords: &str, limit: u32, offset: u32) -> Result<Vec<Song>> {
        let v = self
            .get(
                "cloudsearch",
                vec![
                    ("keywords", keywords.to_string()),
                    ("type", "1".into()),
                    ("limit", limit.to_string()),
                    ("offset", offset.to_string()),
                ],
            )
            .await?;
        let songs = v
            .pointer("/result/songs")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(dto::song_from).collect::<Vec<_>>())
            .unwrap_or_default();
        if songs.is_empty() {
            // 回退旧接口（字段是 artists / album）
            let v = self
                .get(
                    "search",
                    vec![
                        ("keywords", keywords.to_string()),
                        ("type", "1".into()),
                        ("limit", limit.to_string()),
                        ("offset", offset.to_string()),
                    ],
                )
                .await?;
            return Ok(v
                .pointer("/result/songs")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(dto::song_from).collect())
                .unwrap_or_default());
        }
        Ok(songs)
    }

    pub async fn search_playlists(&self, keywords: &str, limit: u32) -> Result<Vec<Playlist>> {
        let v = self
            .get(
                "cloudsearch",
                vec![
                    ("keywords", keywords.to_string()),
                    ("type", "1000".into()),
                    ("limit", limit.to_string()),
                ],
            )
            .await?;
        Ok(v.pointer("/result/playlists")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(dto::playlist_from).collect())
            .unwrap_or_default())
    }

    pub async fn song_detail(&self, ids: &[String]) -> Result<Vec<Song>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        // 一次请求太多 ID 会让 URL 过长，分块查询并保持原顺序。
        const CHUNK: usize = 100;
        let mut songs = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(CHUNK) {
            let v = self
                .get("song/detail", vec![("ids", chunk.join(","))])
                .await?;
            songs.extend(
                dto::array_of(&v, &["songs"])
                    .iter()
                    .filter_map(dto::song_from),
            );
        }
        Ok(songs)
    }

    pub async fn song_url(&self, id: &str, quality: Quality) -> Result<Option<SongUrl>> {
        let mut params = vec![
            ("id", id.to_string()),
            ("level", quality.level().to_string()),
        ];
        // realIP 只用于播放链接；搜索等接口带上它反而容易被风控拒绝。
        if let Some(ip) = &self.real_ip {
            params.push(("realIP", ip.clone()));
        }
        let v = self.get("song/url/v1", params).await?;
        let Some(item) = v
            .get("data")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
        else {
            return Ok(None);
        };
        let Some(url) = item.get("url").and_then(Value::as_str) else {
            return Ok(None);
        };
        if url.trim().is_empty() {
            return Ok(None);
        }
        let url = if let Some(rest) = url.strip_prefix("http://") {
            format!("https://{rest}")
        } else {
            url.to_string()
        };
        let format = item
            .get("type")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .or_else(|| {
                url.split('?')
                    .next()
                    .and_then(|p| p.rsplit('.').next())
                    .map(|s| s.to_ascii_lowercase())
            })
            .unwrap_or_else(|| "mp3".into());
        let trial = item.get("freeTrialInfo").is_some_and(|v| !v.is_null());
        Ok(Some(SongUrl {
            url,
            br: item.get("br").and_then(Value::as_u64).unwrap_or(0),
            size: item.get("size").and_then(Value::as_u64).unwrap_or(0),
            format,
            trial,
        }))
    }

    pub async fn lyric(&self, id: &str) -> Result<RawLyric> {
        let v = match self.get("lyric/new", vec![("id", id.to_string())]).await {
            Ok(v) => v,
            Err(_) => {
                let old = self.get("lyric", vec![("id", id.to_string())]).await?;
                return Ok(RawLyric {
                    lrc: old
                        .pointer("/lrc/lyric")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    tlyric: old
                        .pointer("/tlyric/lyric")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                });
            }
        };
        Ok(RawLyric {
            lrc: v
                .pointer("/lrc/lyric")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            tlyric: v
                .pointer("/tlyric/lyric")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        })
    }

    pub async fn playlist_meta(&self, id: &str) -> Result<Playlist> {
        let v = self
            .get("playlist/detail", vec![("id", id.to_string())])
            .await?;
        v.pointer("/playlist")
            .and_then(dto::playlist_from)
            .ok_or_else(|| Error::Api("歌单不存在".into()))
    }

    pub async fn playlist_tracks(&self, id: &str, limit: u32, offset: u32) -> Result<Vec<Song>> {
        // `/playlist/track/all` 一次拉上千首很容易被上游重置，按 100 首分页。
        const PAGE: u32 = 100;
        let limit = limit.max(1);
        let mut songs = Vec::new();
        let mut offset = offset;
        while (songs.len() as u32) < limit {
            let want = PAGE.min(limit - songs.len() as u32);
            let v = self
                .get(
                    "playlist/track/all",
                    vec![
                        ("id", id.to_string()),
                        ("limit", want.to_string()),
                        ("offset", offset.to_string()),
                    ],
                )
                .await?;
            let page = dto::array_of(&v, &["songs"]);
            let count = page.len();
            songs.extend(page.iter().filter_map(dto::song_from));
            if count < want as usize {
                break;
            }
            offset += count as u32;
        }
        songs.truncate(limit as usize);
        Ok(songs)
    }

    /// 网易原创榜前 N 首（榜单 ID 2884035）。
    pub async fn original_chart(&self, limit: u32) -> Result<Vec<Song>> {
        self.playlist_tracks("2884035", limit, 0).await
    }

    /// 雀乐最新 10 期，歌曲随歌单一起返回。
    pub async fn indie_catalog(&self) -> Result<Vec<dto::IndiePlaylist>> {
        let v = self.get_local("musicyi/catalog/indie", vec![]).await?;
        Ok(dto::array_of(&v, &["playlists"])
            .iter()
            .filter_map(dto::indie_playlist_from)
            .collect())
    }

    async fn streetvoice_list(&self, kind: &str) -> Result<Vec<Song>> {
        let v = self
            .get_local(&format!("musicyi/catalog/streetvoice/{kind}"), vec![])
            .await?;
        Ok(dto::array_of(&v, &["songs"])
            .iter()
            .filter_map(|x| dto::source_song_from(x, SourceKind::StreetVoice))
            .collect())
    }

    /// 街声即时榜（前端已不使用）。
    pub async fn streetvoice_realtime(&self) -> Result<Vec<Song>> {
        self.streetvoice_list("realtime").await
    }

    /// 街声推荐前 30 首。
    pub async fn streetvoice_recommend(&self) -> Result<Vec<Song>> {
        self.streetvoice_list("recommend").await
    }

    pub async fn streetvoice_detail(&self, id: &str) -> Result<StreetVoiceDetail> {
        let v = self
            .get_local(&format!("musicyi/catalog/streetvoice/{id}"), vec![])
            .await?;
        Ok(StreetVoiceDetail {
            duration_ms: v.get("duration_ms").and_then(Value::as_u64).unwrap_or(0),
            album: v
                .get("album")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            cover_url: v
                .get("cover_url")
                .and_then(Value::as_str)
                .map(str::to_string),
            lyrics: v
                .get("lyrics")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            lyrics_is_lrc: v
                .get("lyrics_is_lrc")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}
