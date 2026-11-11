//! 宽松的 DTO 映射。
//!
//! netease-cloud-music-api 的不同接口返回字段名不一致（`ar/al/dt` 与
//! `artists/album/duration` 并存），这里统一在 JSON 层做兼容。

use musicyi_core::model::{Playlist, Song, SourceKind};
use serde_json::Value;

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(|x| x.to_string())
}

fn id_of(v: &Value) -> Option<String> {
    match v.get("id") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => v.get("id").map(|x| x.to_string()),
    }
}

fn u64_of(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(|x| {
        x.as_u64()
            .or_else(|| x.as_i64().and_then(|i| u64::try_from(i).ok()))
            .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
    })
}

fn i64_of(v: &Value, key: &str) -> i64 {
    v.get(key)
        .and_then(|x| {
            x.as_i64()
                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0)
}

fn https(url: Option<String>) -> Option<String> {
    url.map(|u| {
        if let Some(rest) = u.strip_prefix("http://") {
            format!("https://{rest}")
        } else {
            u
        }
    })
}

/// 兼容 `ar` / `artists` 两种歌手字段。
fn artists_of(v: &Value) -> Vec<String> {
    let arr = v
        .get("ar")
        .or_else(|| v.get("artists"))
        .and_then(|x| x.as_array());
    arr.map(|items| {
        items
            .iter()
            .filter_map(|a| s(a, "name"))
            .filter(|n| !n.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

/// 兼容 `al` / `album` 两种专辑字段。
fn album_of(v: &Value) -> (String, Option<String>, Option<String>) {
    let al = v.get("al").or_else(|| v.get("album"));
    match al {
        Some(al) => (
            s(al, "name").unwrap_or_default(),
            id_of(al),
            https(
                s(al, "picUrl")
                    .or_else(|| s(al, "pic"))
                    .or_else(|| s(al, "img1v1Url")),
            ),
        ),
        None => (String::new(), None, None),
    }
}

pub fn song_from(v: &Value) -> Option<Song> {
    let id = id_of(v)?;
    let name = s(v, "name")?;
    let (album, album_id, cover) = album_of(v);
    let duration = u64_of(v, "dt")
        .or_else(|| u64_of(v, "duration"))
        .unwrap_or(0);
    Some(Song {
        id,
        name,
        artists: artists_of(v),
        album,
        album_id,
        duration_ms: duration,
        cover_url: https(s(v, "picUrl")).or(cover),
        fee: i64_of(v, "fee"),
        source: SourceKind::Netease,
        play_url: None,
        lyric_url: None,
        origin: None,
    })
}

pub fn playlist_from(v: &Value) -> Option<Playlist> {
    Some(Playlist {
        id: id_of(v)?,
        name: s(v, "name")?,
        cover_url: https(s(v, "coverImgUrl").or_else(|| s(v, "picUrl"))),
        track_count: u64_of(v, "trackCount").unwrap_or(0),
        play_count: u64_of(v, "playCount").unwrap_or(0),
        creator: v
            .get("creator")
            .and_then(|c| s(c, "nickname"))
            .or_else(|| s(v, "copywriter"))
            .unwrap_or_default(),
        description: s(v, "description").unwrap_or_default(),
    })
}

/// 雀乐一期歌单：元信息与歌曲一次性返回，打开时无需再请求。
#[derive(Debug, Clone, Default)]
pub struct IndiePlaylist {
    pub id: String,
    pub name: String,
    pub cover_url: Option<String>,
    pub track_count: u64,
    pub songs: Vec<Song>,
}

/// 本机 API 自己整理的来源（雀乐 / 街声）字段更规整，直接映射。
pub fn source_song_from(v: &Value, source: SourceKind) -> Option<Song> {
    let id = id_of(v)?;
    let name = s(v, "name").or_else(|| s(v, "title"))?;
    let artists = v
        .get("artists")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(Song {
        id,
        name,
        artists,
        album: s(v, "album").unwrap_or_default(),
        album_id: None,
        duration_ms: u64_of(v, "duration_ms")
            .or_else(|| u64_of(v, "length").map(|s| s.saturating_mul(1000)))
            .unwrap_or(0),
        cover_url: https(
            s(v, "cover_url")
                .or_else(|| s(v, "pic"))
                .or_else(|| s(v, "image")),
        ),
        fee: 0,
        source,
        play_url: https(s(v, "play_url").or_else(|| s(v, "src"))),
        lyric_url: https(s(v, "lyric_url").or_else(|| s(v, "lrc"))),
        origin: None,
    })
}

pub fn indie_playlist_from(v: &Value) -> Option<IndiePlaylist> {
    let songs = v
        .get("songs")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|x| source_song_from(x, SourceKind::Indie))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(IndiePlaylist {
        id: id_of(v)?,
        name: s(v, "name")?,
        cover_url: https(s(v, "cover_url")),
        track_count: u64_of(v, "track_count").unwrap_or(songs.len() as u64),
        songs,
    })
}

/// 从响应中取第一个存在的数组字段（`songs` / `result` / `data` 等）。
pub fn array_of<'a>(v: &'a Value, keys: &[&str]) -> &'a [Value] {
    for key in keys {
        if let Some(arr) = v.get(*key).and_then(|x| x.as_array()) {
            return arr;
        }
    }
    &[]
}
