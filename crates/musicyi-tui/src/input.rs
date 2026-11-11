//! 输入分类：把用户敲进搜索框的一行字归到最合适的打开方式。
//!
//! 顺序很重要——先看磁盘上的路径，再看网易云链接，最后才当关键词搜索。

use std::path::{Path, PathBuf};

use musicyi_core::util::expand_home;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Classified {
    /// 一个存在的 `.m3u` / `.m3u8` 文件。
    M3u(PathBuf),
    /// 本地音频文件（可能不存在，交给上层报错）。
    Local(PathBuf),
    /// 网易云歌单 ID。
    Playlist(String),
    /// 网易云歌曲 ID。
    Song(String),
    /// 普通搜索关键词。
    Query(String),
}

const AUDIO_EXTS: [&str; 15] = [
    "mp3", "flac", "m4a", "mp4", "aac", "ogg", "opus", "wav", "aiff", "aif", "alac", "wv", "ape",
    "mka", "m3u",
];

pub fn classify(raw: &str) -> Classified {
    let text = raw.trim();
    if text.is_empty() {
        return Classified::Query(String::new());
    }

    let path = expand_home(text);
    if is_m3u_path(&path) {
        return Classified::M3u(path);
    }
    if is_audio_path(&path) {
        return Classified::Local(path);
    }

    if let Some(id) = extract_playlist_id(text) {
        return Classified::Playlist(id);
    }
    if let Some(id) = extract_song_id(text) {
        return Classified::Song(id);
    }

    if starts_like_path(text) {
        return Classified::Local(path);
    }

    Classified::Query(text.to_string())
}

pub fn is_m3u_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("m3u") || e.eq_ignore_ascii_case("m3u8"))
}

pub fn is_audio_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

pub fn looks_like_audio(text: &str) -> bool {
    is_audio_path(Path::new(text.trim()))
}

/// 从网易云歌曲链接或纯数字 ID 里取歌曲 ID。
pub fn extract_song_id(raw: &str) -> Option<String> {
    let text = raw.trim();
    if is_all_digits(text) {
        return Some(text.to_string());
    }
    let lower = text.to_ascii_lowercase();
    if !lower.contains("music.163.com") {
        return None;
    }
    if lower.contains("playlist") || lower.contains("toplist") {
        return None;
    }
    if lower.contains("/song") {
        return query_param(text, "id")
            .or_else(|| digits_after(text, "/song/"))
            .filter(|s| !s.is_empty());
    }
    None
}

/// 从网易云歌单链接里取歌单 ID。
pub fn extract_playlist_id(raw: &str) -> Option<String> {
    let text = raw.trim();
    let lower = text.to_ascii_lowercase();
    if !lower.contains("music.163.com") {
        return None;
    }
    if !(lower.contains("playlist") || lower.contains("toplist")) {
        return None;
    }
    query_param(text, "id")
        .or_else(|| digits_after(text, "/playlist/"))
        .or_else(|| digits_after(text, "/toplist/"))
        .filter(|s| !s.is_empty())
}

fn is_all_digits(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit())
}

fn starts_like_path(text: &str) -> bool {
    text.starts_with('/')
        || text.starts_with('~')
        || text.starts_with("./")
        || text.starts_with("../")
}

/// 取 URL 查询串里的某个参数，只保留数字部分。
fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split('?').nth(1)?;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=')?;
        if k == key {
            let digits: String = v.chars().take_while(|c| c.is_ascii_digit()).collect();
            return Some(digits);
        }
    }
    None
}

fn digits_after(text: &str, marker: &str) -> Option<String> {
    let idx = text.to_ascii_lowercase().find(marker)? + marker.len();
    let digits: String = text[idx..]
        .chars()
        .skip_while(|c| *c == '/')
        .take_while(|c| c.is_ascii_digit())
        .collect();
    Some(digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn song_urls_and_ids() {
        assert_eq!(
            extract_song_id("https://music.163.com/#/song?id=1858110236").as_deref(),
            Some("1858110236")
        );
        assert_eq!(
            extract_song_id("https://music.163.com/song/1858110236").as_deref(),
            Some("1858110236")
        );
        assert_eq!(extract_song_id("1858110236").as_deref(), Some("1858110236"));
        assert_eq!(extract_song_id("晴天"), None);
    }

    #[test]
    fn playlist_urls() {
        assert_eq!(
            extract_playlist_id("https://music.163.com/#/playlist?id=3778678&userid=1").as_deref(),
            Some("3778678")
        );
        assert_eq!(
            extract_playlist_id("https://music.163.com/playlist/3778678").as_deref(),
            Some("3778678")
        );
        assert_eq!(
            extract_playlist_id("https://music.163.com/#/song?id=42"),
            None
        );
    }

    #[test]
    fn classify_prefers_paths() {
        assert_eq!(
            classify("/tmp/a.m3u"),
            Classified::M3u(PathBuf::from("/tmp/a.m3u"))
        );
        assert_eq!(
            classify("./song.flac"),
            Classified::Local(PathBuf::from("./song.flac"))
        );
        assert_eq!(
            classify("https://music.163.com/#/playlist?id=3778678"),
            Classified::Playlist("3778678".into())
        );
        assert_eq!(
            classify("https://music.163.com/#/song?id=42"),
            Classified::Song("42".into())
        );
        assert_eq!(classify("42"), Classified::Song("42".into()));
        assert_eq!(classify("周杰伦"), Classified::Query("周杰伦".into()));
        assert_eq!(classify(""), Classified::Query(String::new()));
    }

    #[test]
    fn nonexistent_path_kept_as_local() {
        assert_eq!(
            classify("~/Music/不存在.mp3"),
            Classified::Local(musicyi_core::util::expand_home("~/Music/不存在.mp3"))
        );
    }
}
