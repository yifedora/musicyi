use std::path::Path;

use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::read_from_path;
use lofty::tag::{Accessor, ItemKey, Tag};

/// 从本地文件读出的标签信息。
#[derive(Debug, Clone, Default)]
pub struct EmbeddedMeta {
    pub title: Option<String>,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub lyrics: Option<String>,
}

/// 读取音频文件的标签；无法解析时返回 None。
pub fn read_meta(path: &Path) -> Option<EmbeddedMeta> {
    let tagged = read_from_path(path).ok()?;
    let duration_ms = tagged.properties().duration().as_millis() as u64;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    let title = tag.and_then(|t| t.title().map(|s| s.into_owned()));
    let album = tag.and_then(|t| t.album().map(|s| s.into_owned()));
    let artists = tag
        .and_then(|t| t.artist().map(|s| s.into_owned()))
        .map(|raw| split_artists(&raw))
        .unwrap_or_default();
    let lyrics = tag
        .and_then(|t| {
            t.get_string(ItemKey::Lyrics)
                .or_else(|| t.get_string(ItemKey::UnsyncLyrics))
                .map(str::to_string)
        })
        .filter(|s| !s.trim().is_empty());

    Some(EmbeddedMeta {
        title,
        artists,
        album,
        duration_ms,
        lyrics,
    })
}

/// 只读取内嵌歌词，供本地歌曲歌词面板使用。
pub fn read_lyrics(path: &Path) -> Option<String> {
    read_meta(path).and_then(|m| m.lyrics)
}

/// 读取内嵌封面（优先 Front Cover，否则第一张图）。
pub fn read_cover(path: &Path) -> Option<Vec<u8>> {
    let tagged = read_from_path(path).ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let picture = tag
        .get_picture_type(PictureType::CoverFront)
        .or_else(|| tag.pictures().first())?;
    Some(picture.data().to_vec())
}

/// 判断文件标签是否和文件名明显不一致（缺失、字段错位、歌手对不上）。
///
/// 本地曲库的文件名普遍是 `歌手 - 标题.mp3`，而内嵌标签有空缺或整组
/// 错位的情况，因此以文件名为准做一次一致性检查，供批量补全挑选目标。
pub fn tag_needs_fix(path: &Path) -> bool {
    let meta = read_meta(path);
    let (hint_artist, hint_title) = filename_hint(path);
    tag_needs_fix_from(meta.as_ref(), hint_artist.as_deref(), hint_title.as_deref())
}

fn tag_needs_fix_from(
    meta: Option<&EmbeddedMeta>,
    hint_artist: Option<&str>,
    hint_title: Option<&str>,
) -> bool {
    let Some(meta) = meta else {
        return true;
    };
    let title_ok = match hint_title.map(str::trim).filter(|s| !s.is_empty()) {
        Some(hint) => meta
            .title
            .as_deref()
            .is_some_and(|title| text_matches(title, hint)),
        None => meta.title.as_deref().is_some_and(|t| !t.trim().is_empty()),
    };
    let artist_ok = match hint_artist.map(str::trim).filter(|s| !s.is_empty()) {
        Some(hint) => meta
            .artists
            .iter()
            .any(|artist| artist != "未知歌手" && text_matches(artist, hint)),
        None => meta
            .artists
            .iter()
            .any(|artist| artist != "未知歌手" && !artist.trim().is_empty()),
    };
    !(title_ok && artist_ok)
}

/// 归一化后比较两个文本，允许一方包含另一方（长度足够时）。
pub fn text_matches(left: &str, right: &str) -> bool {
    let left = normalize_text(left);
    let right = normalize_text(right);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left == right {
        return true;
    }
    let short = left.chars().count().min(right.chars().count());
    short >= 4 && (left.contains(&right) || right.contains(&left))
}

fn normalize_text(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// 从文件名猜 `歌手 - 标题`。标签缺失或明显错误时作为兜底线索。
pub fn filename_hint(path: &Path) -> (Option<String>, Option<String>) {
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return (None, None);
    };
    let stem = strip_track_prefix(stem.trim());
    for sep in [" - ", " – ", " — "] {
        if let Some((artist, title)) = stem.split_once(sep) {
            let artist = artist.trim();
            let title = title.trim();
            if !artist.is_empty() && !title.is_empty() {
                return (Some(artist.to_string()), Some(title.to_string()));
            }
        }
    }
    (None, Some(stem.to_string()).filter(|s| !s.is_empty()))
}

fn strip_track_prefix(stem: &str) -> &str {
    let digits = stem.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 3 {
        return stem;
    }
    let rest = stem[digits..]
        .trim_start_matches(['.', '-', '_', ' '])
        .trim();
    if rest.is_empty() { stem } else { rest }
}

/// 把补全后的标题、歌手、专辑写回音频文件。
pub fn write_meta(
    path: &Path,
    title: Option<&str>,
    artists: &[String],
    album: Option<&str>,
) -> musicyi_core::Result<()> {
    write_meta_full(path, title, artists, album, None, None)
}

/// 写入标题、歌手、专辑，并可选地嵌入歌词与封面。
pub fn write_meta_full(
    path: &Path,
    title: Option<&str>,
    artists: &[String],
    album: Option<&str>,
    lyrics: Option<&str>,
    cover: Option<&[u8]>,
) -> musicyi_core::Result<()> {
    let mut tagged = read_from_path(path)
        .map_err(|e| musicyi_core::Error::Storage(format!("读取标签失败: {e}")))?;
    if tagged.primary_tag_mut().is_none() {
        let tag_type = tagged.primary_tag_type();
        tagged.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged
        .primary_tag_mut()
        .ok_or_else(|| musicyi_core::Error::Storage("文件不支持写入标签".into()))?;
    if let Some(title) = title.map(str::trim).filter(|s| !s.is_empty()) {
        tag.set_title(title.to_string());
    }
    let artist = artists
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" / ");
    if !artist.is_empty() {
        tag.set_artist(artist);
    }
    if let Some(album) = album.map(str::trim).filter(|s| !s.is_empty()) {
        tag.set_album(album.to_string());
    }
    if let Some(text) = lyrics.map(str::trim).filter(|s| !s.is_empty()) {
        // ID3 的 USLT 对应 UnsyncLyrics；部分格式同时映射 Lyrics。
        tag.remove_key(ItemKey::Lyrics);
        tag.remove_key(ItemKey::UnsyncLyrics);
        tag.insert_text(ItemKey::UnsyncLyrics, text.to_string());
    }
    if let Some(data) = cover.filter(|d| !d.is_empty()) {
        let mime = if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            MimeType::Jpeg
        } else if data.starts_with(&[0x89, b'P', b'N', b'G']) {
            MimeType::Png
        } else {
            MimeType::Unknown("application/octet-stream".into())
        };
        let picture = Picture::unchecked(data.to_vec())
            .pic_type(PictureType::CoverFront)
            .mime_type(mime)
            .build();
        tag.remove_picture_type(PictureType::CoverFront);
        tag.push_picture(picture);
    }
    tagged
        .save_to_path(path, WriteOptions::default())
        .map_err(|e| musicyi_core::Error::Storage(format!("写入标签失败: {e}")))
}

fn split_artists(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = raw
        .split(['/', ';', '、'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    out.dedup();
    if out.is_empty() {
        out.push("未知歌手".to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{EmbeddedMeta, filename_hint, tag_needs_fix_from, text_matches};
    use std::path::Path;

    fn meta(title: &str, artists: &[&str], album: &str) -> EmbeddedMeta {
        EmbeddedMeta {
            title: Some(title.to_string()),
            artists: artists.iter().map(|a| a.to_string()).collect(),
            album: Some(album.to_string()),
            ..EmbeddedMeta::default()
        }
    }

    #[test]
    fn filename_artist_title() {
        let (artist, title) = filename_hint(Path::new("/tmp/April Rain - Ground Zero.mp3"));
        assert_eq!(artist.as_deref(), Some("April Rain"));
        assert_eq!(title.as_deref(), Some("Ground Zero"));
    }

    #[test]
    fn filename_without_artist() {
        let (artist, title) = filename_hint(Path::new("/tmp/01. Untitled.mp3"));
        assert_eq!(artist, None);
        assert_eq!(title.as_deref(), Some("Untitled"));
    }

    #[test]
    fn good_tags_do_not_need_fix() {
        let tags = meta("Ground Zero", &["April Rain"], "Undertone");
        assert!(!tag_needs_fix_from(
            Some(&tags),
            Some("April Rain"),
            Some("Ground Zero")
        ));
    }

    #[test]
    fn rotated_tags_need_fix() {
        let tags = meta("April Rain", &["Undertone"], "Ground Zero");
        assert!(tag_needs_fix_from(
            Some(&tags),
            Some("April Rain"),
            Some("Ground Zero")
        ));
    }

    #[test]
    fn missing_tags_need_fix() {
        assert!(tag_needs_fix_from(
            None,
            Some("April Rain"),
            Some("Ground Zero")
        ));
        let empty = EmbeddedMeta::default();
        assert!(tag_needs_fix_from(
            Some(&empty),
            Some("April Rain"),
            Some("Ground Zero")
        ));
    }

    #[test]
    fn text_match_handles_case_and_spacing() {
        assert!(text_matches("Gun Song", "gun-song"));
        assert!(text_matches("The Big Pink", "The Big Pink"));
        assert!(!text_matches("To", "Tomorrow"));
    }
}
