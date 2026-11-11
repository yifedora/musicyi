//! M3U / M3U8 解析。
//!
//! 同时支持本地路径与网易云歌曲链接/ID。相对路径按歌单文件所在目录解析。
//! 这里不读网络、不读标签，只负责把文本变成条目。

use std::path::{Path, PathBuf};

use crate::input;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum M3uEntry {
    Local {
        path: PathBuf,
        title: Option<String>,
        duration_ms: u64,
    },
    Online {
        id: String,
        title: Option<String>,
    },
}

pub fn parse(text: &str, base: &Path) -> Vec<M3uEntry> {
    let mut out = Vec::new();
    let mut pending: Option<(Option<String>, u64)> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.eq_ignore_ascii_case("#EXTM3U") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let (dur, title) = match rest.split_once(',') {
                Some((d, t)) => (d.trim(), Some(t.trim().to_string())),
                None => (rest.trim(), None),
            };
            let seconds = dur.parse::<u64>().unwrap_or(0);
            pending = Some((
                title.filter(|t| !t.is_empty()),
                seconds.saturating_mul(1000),
            ));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }

        let (title, duration_ms) = pending.take().unwrap_or((None, 0));
        if let Some(entry) = entry_for(line, base, title, duration_ms) {
            out.push(entry);
        }
    }
    out
}

fn entry_for(line: &str, base: &Path, title: Option<String>, duration_ms: u64) -> Option<M3uEntry> {
    // 先当本地路径：绝对路径原样，相对路径拼到 M3U 所在目录。
    if !looks_remote(line) {
        let raw = Path::new(line);
        let path = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            base.join(raw)
        };
        if input::is_audio_path(&path) {
            return Some(M3uEntry::Local {
                path,
                title,
                duration_ms,
            });
        }
    }

    if let Some(id) = input::extract_song_id(line) {
        return Some(M3uEntry::Online { id, title });
    }
    None
}

fn looks_remote(line: &str) -> bool {
    line.contains("://") || line.to_ascii_lowercase().contains("music.163.com")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_extinf_and_relative_paths() {
        let raw = "#EXTM3U\n#EXTINF:123,Artist - Song\n./a.mp3\n#EXTINF:-1,Second\nb.flac\n";
        let entries = parse(raw, Path::new("/music/list"));
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0],
            M3uEntry::Local {
                path: PathBuf::from("/music/list/a.mp3"),
                title: Some("Artist - Song".into()),
                duration_ms: 123_000,
            }
        );
        assert_eq!(
            entries[1],
            M3uEntry::Local {
                path: PathBuf::from("/music/list/b.flac"),
                title: Some("Second".into()),
                duration_ms: 0,
            }
        );
    }

    #[test]
    fn parses_netease_urls_and_ids() {
        let raw = "#EXTM3U\nhttps://music.163.com/#/song?id=42\n#EXTINF:10,Name\n10086\n";
        let entries = parse(raw, Path::new("/tmp"));
        assert_eq!(
            entries[0],
            M3uEntry::Online {
                id: "42".into(),
                title: None,
            }
        );
        assert_eq!(
            entries[1],
            M3uEntry::Online {
                id: "10086".into(),
                title: Some("Name".into()),
            }
        );
    }

    #[test]
    fn ignores_unknown_lines() {
        let raw = "#EXTM3U\n# just a comment\nnot-a-song\n";
        assert!(parse(raw, Path::new(".")).is_empty());
    }
}
