//! 网易云缺歌时的外部回退：
//! - 歌词：LRCLIB（https://lrclib.net）
//! - 元数据 / 封面：MusicBrainz（CC0，1 req/s 限速）+ Cover Art Archive
//! - 全部失败时把文件名追加到日志目录的 `tag-failures.log`
//!
//! 每个请求都有独立超时；任何失败都不会中断批量任务，只计入失败计数。

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

const LRCLIB_BASE: &str = "https://lrclib.net";
const MB_BASE: &str = "https://musicbrainz.org/ws/2";
const CAA_BASE: &str = "https://coverartarchive.org/release";
const MB_UA: &str = "musicyi/0.1 (https://github.com/yifedora/musicyi)";
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

/// 面向外部库（MusicBrainz / LRCLIB / Cover Art Archive）的专用客户端。
/// 不信任环境变量代理：外部库网络可达性已知直连可用，环境里若有失败的
/// HTTPS_PROXY 会让所有请求被劫持；网易云仍走主客户端（可能需要代理）。
pub fn external_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(MB_UA)
        .no_proxy()
        .build()
        .map_err(|e| format!("构建外部库客户端失败: {e}"))
}

/// 统一比较键：繁→简 + 小写 + 只留字母数字。与 `app::normalize` 等价，
/// 这里复制一份避免把私有函数暴露出去。
fn key(text: &str) -> String {
    use std::sync::LazyLock;
    static CC: LazyLock<Option<ferrous_opencc::OpenCC>> = LazyLock::new(|| {
        ferrous_opencc::OpenCC::from_config(ferrous_opencc::config::BuiltinConfig::T2s).ok()
    });
    let converted = CC
        .as_ref()
        .map(|cc| cc.convert(text))
        .unwrap_or_else(|| text.to_string());
    converted
        .chars()
        .flat_map(|c| c.to_lowercase())
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// 时长容差：本地文件与网易云/MusicBrainz 的时长差限值（毫秒）。
const DURATION_TOL_MS: u64 = 10_000;

/// 从 LRCLIB 搜歌词。返回 LRC（含时间轴）或纯文本。
pub async fn lrclib_lyrics(
    http: &reqwest::Client,
    title: &str,
    artist: &str,
    duration_ms: u64,
) -> Option<String> {
    let track = title.trim();
    if track.is_empty() {
        return None;
    }
    let mut url = format!("{LRCLIB_BASE}/api/search?track_name={}", urlencode(track));
    if !artist.trim().is_empty() {
        url.push_str(&format!("&artist_name={}", urlencode(artist.trim())));
    }
    let resp = http
        .get(&url)
        .header("User-Agent", MB_UA)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let items: Vec<LrclibTrack> = resp.json().await.ok()?;
    pick_lrclib(&items, title, artist, duration_ms)
}

#[derive(Debug, Deserialize)]
struct LrclibTrack {
    #[serde(rename = "trackName")]
    track_name: String,
    #[serde(rename = "artistName", default)]
    artist_name: String,
    #[serde(default)]
    duration: f64,
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
}

fn pick_lrclib(
    items: &[LrclibTrack],
    title: &str,
    artist: &str,
    duration_ms: u64,
) -> Option<String> {
    let want_title = key(title);
    let want_artist = key(artist);
    let mut best: Option<(i64, &LrclibTrack)> = None;
    for item in items {
        let title_k = key(&item.track_name);
        if title_k.is_empty()
            || !(title_k == want_title
                || title_k.contains(&want_title)
                || want_title.contains(&title_k))
        {
            continue;
        }
        if !want_artist.is_empty() {
            let artist_k = key(&item.artist_name);
            if !(artist_k == want_artist
                || artist_k.contains(&want_artist)
                || want_artist.contains(&artist_k))
            {
                continue;
            }
        }
        let dur_diff = if duration_ms > 0 && item.duration > 0.0 {
            (duration_ms as f64 - item.duration * 1000.0).abs() as i64
        } else {
            0
        };
        if dur_diff > DURATION_TOL_MS as i64 {
            continue;
        }
        // 优先同步歌词；同一档取时长差最小的
        let rank = if item
            .synced_lyrics
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            1
        } else {
            0
        };
        let score = rank * 1_000_000 + dur_diff;
        if best.is_none_or(|(s, _)| score < s) {
            best = Some((score, item));
        }
    }
    let (_, item) = best?;
    let synced = item.synced_lyrics.as_deref().unwrap_or("").trim();
    if !synced.is_empty() {
        Some(synced.to_string())
    } else {
        let plain = item.plain_lyrics.as_deref().unwrap_or("").trim();
        if plain.is_empty() {
            None
        } else {
            Some(plain.to_string())
        }
    }
}

fn urlencode(s: &str) -> String {
    // 只保留 unreserved 字符，其余按 UTF-8 字节 percent-encode
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// MusicBrainz 元数据 + Cover Art Archive 封面。不含歌词。
#[derive(Debug, Clone)]
pub struct FallbackMeta {
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub cover: Option<Vec<u8>>,
}

pub async fn musicbrainz_meta(
    http: &reqwest::Client,
    title: &str,
    artist: &str,
    duration_ms: u64,
) -> Option<FallbackMeta> {
    let query = if artist.trim().is_empty() {
        format!("recording:\"{}\"", title.trim())
    } else {
        format!(
            "recording:\"{}\" AND artist:\"{}\"",
            title.trim(),
            artist.trim()
        )
    };
    let url = format!(
        "{MB_BASE}/recording?query={}&fmt=json&limit=5",
        urlencode(&query)
    );
    let resp = http
        .get(&url)
        .header("User-Agent", MB_UA)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let value: Value = resp.json().await.ok()?;
    let recordings = value.get("recordings")?.as_array()?;

    let want_title = key(title);
    let want_artist = key(artist);
    let mut best: Option<(i64, &Value)> = None;
    for rec in recordings {
        let Some(rec_title) = rec.get("title").and_then(Value::as_str) else {
            continue;
        };
        let title_k = key(rec_title);
        if title_k.is_empty()
            || !(title_k == want_title
                || title_k.contains(&want_title)
                || want_title.contains(&title_k))
        {
            continue;
        }
        let artist_name = rec
            .get("artist-credit")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if !want_artist.is_empty() {
            let ak = key(&artist_name);
            if !(ak == want_artist || ak.contains(&want_artist) || want_artist.contains(&ak)) {
                continue;
            }
        }
        let dur_diff = match (duration_ms, rec.get("length").and_then(Value::as_u64)) {
            (t, Some(l)) if t > 0 => (t as i64 - l as i64).abs(),
            _ => 0,
        };
        if dur_diff > DURATION_TOL_MS as i64 {
            continue;
        }
        let exact = if title_k == want_title { 0 } else { 100_000 };
        let score = exact + dur_diff;
        if best.is_none_or(|(s, _)| score < s) {
            best = Some((score, rec));
        }
    }
    let (_, rec) = best?;
    let title_out = rec.get("title").and_then(Value::as_str)?.to_string();
    let artist_out = rec
        .get("artist-credit")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.get("name").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" / ")
        })
        .unwrap_or_else(|| artist.to_string());
    let release = rec
        .get("releases")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first());
    let album = release
        .and_then(|r| r.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let release_id = release
        .and_then(|r| r.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let cover = match release_id {
        Some(id) => cover_art(http, &id).await,
        None => None,
    };
    Some(FallbackMeta {
        title: title_out,
        artists: vec![artist_out],
        album,
        cover,
    })
}

async fn cover_art(http: &reqwest::Client, release_mbid: &str) -> Option<Vec<u8>> {
    let url = format!("{CAA_BASE}/{release_mbid}/front-500");
    let resp = http.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    if bytes.is_empty() {
        None
    } else {
        Some(bytes.to_vec())
    }
}

/// 把失败文件追加到 `tag-failures.log`（TSV）。
pub fn log_failure(log_dir: &Path, origin: &Path, query: &str, reason: &str) {
    let path: PathBuf = log_dir.join("tag-failures.log");
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let line = format!(
        "{ts}\t{}\t{}\t{}\n",
        origin.display(),
        query.replace(['\t', '\n'], " "),
        reason.replace(['\t', '\n'], " "),
    );
    // 纯追加；失败路径本身也不该让补全任务崩溃
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let _ = file.write_all(line.as_bytes());
    }
}
