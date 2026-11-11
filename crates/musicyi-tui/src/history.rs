//! 纯文本播放历史：每行一个 `{"ts": ..., "song": {...}}`。
//!
//! 读取时从后往前按 `song.key()` 去重，返回最近 N 首。

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

use musicyi_core::model::Song;
use musicyi_core::util::now_ts;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    ts: i64,
    song: Song,
}

pub fn append(path: &Path, song: &Song) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let entry = Entry {
        ts: now_ts(),
        song: song.clone(),
    };
    let mut line = serde_json::to_string(&entry)?;
    line.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(line.as_bytes())
}

/// 删除某首歌的全部历史记录，返回删除的行数。
pub fn remove(path: &Path, key: &str) -> std::io::Result<usize> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(0);
    };
    let mut kept = String::with_capacity(text.len());
    let mut removed = 0usize;
    for line in text.lines() {
        let matched = serde_json::from_str::<Entry>(line)
            .map(|entry| entry.song.key() == key)
            .unwrap_or(false);
        if matched {
            removed += 1;
        } else {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    if removed > 0 {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("history.jsonl");
        let tmp = parent.join(format!(".{name}.tmp-{}", std::process::id()));
        std::fs::write(&tmp, kept)?;
        std::fs::rename(&tmp, path)?;
    }
    Ok(removed)
}

pub fn recent(path: &Path, limit: usize) -> Vec<Song> {
    if limit == 0 {
        return Vec::new();
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in text.lines().rev() {
        if out.len() >= limit {
            break;
        }
        let Ok(entry) = serde_json::from_str::<Entry>(line) else {
            continue;
        };
        if seen.insert(entry.song.key()) {
            out.push(entry.song);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use musicyi_core::model::SourceKind;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_file(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "musicyi-history-{}-{tag}-{nanos}.jsonl",
            std::process::id()
        ))
    }

    fn song(id: &str, name: &str) -> Song {
        Song {
            id: id.into(),
            name: name.into(),
            source: SourceKind::Netease,
            ..Song::default()
        }
    }

    #[test]
    fn appends_and_dedupes() {
        let path = temp_file("dedupe");
        let a = song("1", "a");
        let b = song("2", "b");
        append(&path, &a).unwrap();
        append(&path, &b).unwrap();
        append(&path, &a).unwrap();

        let recent = recent(&path, 10);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].id, "1");
        assert_eq!(recent[1].id, "2");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn respects_limit_and_skips_bad_lines() {
        let path = temp_file("limit");
        for i in 0..5 {
            append(&path, &song(&i.to_string(), "x")).unwrap();
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();

        let recent = recent(&path, 3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].id, "4");
        assert_eq!(recent[2].id, "2");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_file_is_empty() {
        assert!(recent(Path::new("/tmp/musicyi-no-such-history-file"), 20).is_empty());
    }

    #[test]
    fn removes_all_matching_entries() {
        let path = temp_file("remove");
        let a = song("1", "a");
        let b = song("2", "b");
        append(&path, &a).unwrap();
        append(&path, &b).unwrap();
        append(&path, &a).unwrap();

        assert_eq!(remove(&path, &a.key()).unwrap(), 2);
        let recent = recent(&path, 10);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].id, "2");
        assert_eq!(remove(&path, &a.key()).unwrap(), 0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn remove_missing_file_returns_zero() {
        let path = temp_file("remove-missing");
        assert_eq!(remove(&path, "whatever").unwrap(), 0);
    }
}
