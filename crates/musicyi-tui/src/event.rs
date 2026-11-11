use std::path::PathBuf;

use musicyi_api::{IndiePlaylist, RawLyric};
use musicyi_core::model::{Playlist, Song};
use musicyi_player::PlayerEvent;
use ratatui::crossterm::event::KeyEvent;

use crate::types::{Action, CoverArt};

/// 标签补全成功后返回给界面的元信息。
#[derive(Debug, Clone)]
pub struct TagUpdate {
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    /// 转换后实际播放的文件。
    pub id: String,
    /// 原始文件，保持不动。
    pub origin: Option<String>,
}

/// 本地目录扫描结果：歌曲列表与需要补全标签的文件（按 `Song.id`）。
#[derive(Debug, Clone, Default)]
pub struct LocalScan {
    pub songs: Vec<Song>,
    pub needs_tag: Vec<String>,
}

#[derive(Debug)]
pub enum Event {
    Key(KeyEvent),
    Resize,

    /// 全局单曲搜索；`autoplay` 用于链接打开后直接接管队列。
    SearchSongs {
        seq: u64,
        query: String,
        autoplay: bool,
        result: Result<Vec<Song>, String>,
    },
    /// 本地 M3U / 链接打开后直接接管队列。
    OpenSongs {
        seq: u64,
        title: String,
        result: Result<Vec<Song>, String>,
    },
    /// 网易原创榜。
    NeteaseChart {
        seq: u64,
        result: Result<Vec<Song>, String>,
    },
    /// 网易云歌单搜索。
    NeteasePlaylistHits {
        seq: u64,
        query: String,
        result: Result<Vec<Playlist>, String>,
    },
    /// 网易云歌单内的歌曲。
    NeteaseTracks {
        seq: u64,
        id: String,
        title: String,
        result: Result<Vec<Song>, String>,
    },
    /// 雀乐最新 10 期（含歌曲）。
    IndieCatalog {
        seq: u64,
        result: Result<Vec<IndiePlaylist>, String>,
    },
    /// 街声推荐。
    StreetVoiceRecommend {
        seq: u64,
        result: Result<Vec<Song>, String>,
    },
    /// 本地目录扫描。
    LocalSongs {
        seq: u64,
        result: Result<LocalScan, String>,
    },

    /// 单首标签补全。
    TagResult {
        seq: u64,
        old_id: String,
        updated: Option<TagUpdate>,
        message: String,
    },
    /// 批量补全进度。
    TagProgress {
        seq: u64,
        done: u64,
        total: u64,
        name: String,
    },
    TagDone {
        seq: u64,
        fixed: u64,
        failed: u64,
    },

    Lyric {
        key: String,
        raw: RawLyric,
    },
    Cover {
        key: String,
        result: Result<CoverArt, String>,
    },
    ApiStatus {
        ok: bool,
        message: String,
    },

    Resolved {
        seq: u64,
        song: Song,
        url: String,
        provider: String,
        format: String,
        br: u64,
        preview: bool,
        duration_ms: Option<u64>,
    },
    CacheProgress {
        seq: u64,
        done: u64,
        total: Option<u64>,
    },
    CacheReady {
        seq: u64,
        song: Song,
        path: PathBuf,
        provider: String,
    },
    PlayError {
        seq: u64,
        message: String,
    },

    DownloadProgress {
        seq: u64,
        done: u64,
        total: Option<u64>,
    },
    DownloadDone {
        seq: u64,
        path: PathBuf,
        bytes: u64,
    },
    DownloadFailed {
        seq: u64,
        message: String,
    },

    Player(PlayerEvent),
    Action(Action),
}
