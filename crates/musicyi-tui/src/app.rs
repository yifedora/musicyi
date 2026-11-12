use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use musicyi_api::{ApiClient, IndiePlaylist, RawLyric, Resolver};
use musicyi_core::config::Config;
use musicyi_core::lyric::Lyrics;
use musicyi_core::model::{PlayMode, Playlist, Quality, Song, SourceKind};
use musicyi_core::{Error, Paths, Result};
use musicyi_player::{AudioCache, AudioEngine, Cmd, PlayerEvent};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::widgets::TableState;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::event::{Event, LocalScan, TagUpdate};
use crate::history;
use crate::input::{self, Classified};
use crate::m3u;
use crate::mpris::{self, MprisCmd, MprisLoop, MprisState};
use crate::theme::THEME;
use crate::types::{Action, CoverArt, Focus, HOME_ENTRIES, InputMode, View};
use crate::ui;

/// 低于这个尺寸时只显示一行提示，避免表格 / 歌词互相挤压成乱码。
const MIN_WIDTH: u16 = 60;
const MIN_HEIGHT: u16 = 20;

#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionFile {
    queue: Vec<Song>,
    index: Option<usize>,
    pos_ms: u64,
}

#[derive(Debug, Clone)]
struct Opened {
    id: String,
    title: String,
    songs: Vec<Song>,
}

#[derive(Debug, Clone)]
struct SearchResults {
    query: String,
    songs: Vec<Song>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum NeteasePage {
    #[default]
    Browse,
    Search,
    Tracks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum IndiePage {
    #[default]
    Playlists,
    Tracks,
}

/// 每个列表在 `selection_memory` 中使用的键；歌单用 ID 区分，返回时能回到原位。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ListKey {
    Home,
    NeteaseBrowse,
    NeteaseSearch,
    NeteaseTracks(String),
    IndiePlaylists,
    IndieTracks(String),
    StreetVoiceRecommend,
    Local,
    Recent,
    Search,
}

pub struct App {
    cfg: Config,
    paths: Paths,
    api: Arc<ApiClient>,
    resolver: Arc<Resolver>,
    http: reqwest::Client,
    cache: AudioCache,
    engine: AudioEngine,
    tx: UnboundedSender<Event>,

    // 当前视图与输入
    view: View,
    input: String,
    input_cursor: usize,
    input_mode: InputMode,
    focus: Focus,
    list_state: TableState,
    /// 每个列表最后停留的位置，切换视图 / 返回时恢复。
    selection_memory: HashMap<ListKey, usize>,
    error: Option<String>,

    // 全局单曲搜索
    search: Option<SearchResults>,
    search_seq: u64,
    search_loading: bool,

    // 网易云
    netease_chart: Vec<Song>,
    netease_chart_seq: u64,
    netease_chart_loading: bool,
    netease_chart_loaded: bool,
    netease_playlist_hits: Vec<Playlist>,
    netease_tracks: Option<Opened>,
    netease_page: NeteasePage,
    netease_query: String,
    netease_seq: u64,
    netease_loading: bool,
    /// 打开歌单时是否来自歌单搜索结果，决定 Esc 返回到哪里。
    netease_back_to_search: bool,

    // 雀乐
    indie_playlists: Vec<IndiePlaylist>,
    indie_tracks: Option<Opened>,
    indie_page: IndiePage,
    indie_seq: u64,
    indie_loading: bool,
    indie_loaded: bool,

    // 街声
    sv_recommend: Vec<Song>,
    sv_recommend_seq: u64,
    sv_recommend_loading: bool,
    sv_recommend_loaded: bool,

    // 本地
    local_songs: Vec<Song>,
    local_needs_tag: HashSet<String>,
    local_seq: u64,
    local_loading: bool,
    local_loaded: bool,

    // 最近播放
    history: Vec<Song>,

    // 打开 M3U / 链接
    open_seq: u64,

    // 播放
    queue: Vec<Song>,
    cursor: Option<usize>,
    current: Option<Song>,
    loaded: bool,
    playing: bool,
    loading_play: bool,
    pos_ms: u64,
    dur_ms: u64,
    volume: f32,
    quality: Quality,
    mode: PlayMode,
    play_seq: u64,
    next_start_ms: u64,
    pending_autoplay: bool,
    /// 连续播放失败次数，达到队列长度或上限后停止，避免无限跳过。
    failed_streak: u32,
    cache_progress: Option<(u64, Option<u64>)>,

    // 歌词
    lyrics: Lyrics,
    lyrics_key: Option<String>,
    show_lyrics: bool,
    /// 歌词纯享模式：歌词面板向上拓展到整个内容区（列表隐藏）。
    lyrics_immersive: bool,
    /// 相对自动滚动位置的偏移，PageUp / PageDown 调整。
    lyrics_scroll: i32,

    // 封面
    cover: Option<(String, Arc<CoverArt>)>,
    cover_cache: HashMap<String, Arc<CoverArt>>,
    cover_seq: u64,

    // 下载
    download_seq: u64,
    downloading: Option<(u64, String)>,
    download_progress: Option<(u64, Option<u64>)>,

    // 本地标签补全
    tag_seq: u64,
    tagging: bool,
    tag_progress: Option<(u64, u64, String)>,

    // 界面状态
    show_help: bool,
    help_scroll: u16,
    toast: Option<(String, Instant)>,
    quit: bool,

    // MPRIS
    mpris_cmd: Option<UnboundedSender<MprisCmd>>,
    mpris_last_sync: Instant,
}

impl App {
    pub fn new(
        cfg: Config,
        paths: Paths,
        initial: Option<String>,
    ) -> Result<(Self, UnboundedReceiver<Event>)> {
        let api = Arc::new(ApiClient::new(&cfg.api)?);
        let resolver = Arc::new(Resolver::new(api.clone()));
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .user_agent("Mozilla/5.0 musicyi/0.1")
            .build()
            .map_err(|e| Error::Network(e.to_string()))?;
        let cache = AudioCache::new(paths.audio_cache.clone(), cfg.player.cache_limit_mb);
        cache.clean_partials();
        let engine = AudioEngine::spawn(cfg.player.volume);
        let history_songs = history::recent(&paths.history_file, 50);

        let session = if cfg.ui.restore_session {
            load_session(&paths.session_file)
        } else {
            None
        };
        let mut queue = Vec::new();
        let mut cursor = None;
        let mut current = None;
        let mut pos_ms = 0u64;
        let mut dur_ms = 0u64;
        if let Some(session) = session
            && !session.queue.is_empty()
        {
            let index = session.index.unwrap_or(0).min(session.queue.len() - 1);
            queue = session.queue;
            cursor = Some(index);
            current = queue.get(index).cloned();
            dur_ms = current.as_ref().map(|s| s.duration_ms).unwrap_or(0);
            pos_ms = if dur_ms > 0 {
                session.pos_ms.min(dur_ms)
            } else {
                session.pos_ms
            };
        }

        let mut list_state = TableState::default();
        list_state.select(Some(0));

        let volume = cfg.player.volume;
        let quality = cfg.player.quality;
        let mode = cfg.player.mode;
        let (tx, rx) = mpsc::unbounded_channel();

        let mut app = Self {
            cfg,
            paths,
            api,
            resolver,
            http,
            cache,
            engine,
            tx,
            view: View::default(),
            input: String::new(),
            input_cursor: 0,
            input_mode: InputMode::Search,
            focus: Focus::List,
            list_state,
            selection_memory: HashMap::new(),
            error: None,
            search: None,
            search_seq: 0,
            search_loading: false,
            netease_chart: Vec::new(),
            netease_chart_seq: 0,
            netease_chart_loading: false,
            netease_chart_loaded: false,
            netease_playlist_hits: Vec::new(),
            netease_tracks: None,
            netease_page: NeteasePage::default(),
            netease_query: String::new(),
            netease_seq: 0,
            netease_loading: false,
            netease_back_to_search: false,
            indie_playlists: Vec::new(),
            indie_tracks: None,
            indie_page: IndiePage::default(),
            indie_seq: 0,
            indie_loading: false,
            indie_loaded: false,
            sv_recommend: Vec::new(),
            sv_recommend_seq: 0,
            sv_recommend_loading: false,
            sv_recommend_loaded: false,
            local_songs: Vec::new(),
            local_needs_tag: HashSet::new(),
            local_seq: 0,
            local_loading: false,
            local_loaded: false,
            history: history_songs,
            open_seq: 0,
            queue,
            cursor,
            current,
            loaded: false,
            playing: false,
            loading_play: false,
            pos_ms,
            dur_ms,
            volume,
            quality,
            mode,
            play_seq: 0,
            next_start_ms: 0,
            pending_autoplay: true,
            failed_streak: 0,
            cache_progress: None,
            lyrics: Lyrics::default(),
            lyrics_key: None,
            show_lyrics: false,
            lyrics_immersive: false,
            lyrics_scroll: 0,
            cover: None,
            cover_cache: HashMap::new(),
            cover_seq: 0,
            download_seq: 0,
            downloading: None,
            download_progress: None,
            tag_seq: 0,
            tagging: false,
            tag_progress: None,
            show_help: false,
            help_scroll: 0,
            toast: None,
            quit: false,
            mpris_cmd: None,
            mpris_last_sync: Instant::now(),
        };

        app.check_api();
        if app.cfg.ui.mpris {
            app.start_mpris();
        }
        app.ensure_home_data();
        // 歌词面板默认打开；恢复会话时也要把当前歌曲的歌词与封面取回来。
        if let Some(song) = app.current.clone() {
            app.load_lyrics(&song);
            app.load_cover(&song);
        }
        if let Some(raw) = initial {
            app.submit_raw(&raw);
        }
        Ok((app, rx))
    }

    pub fn sender(&self) -> UnboundedSender<Event> {
        self.tx.clone()
    }

    pub fn quit(&self) -> bool {
        self.quit
    }

    fn check_api(&self) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = api.ping().await;
            let _ = tx.send(Event::ApiStatus {
                ok: result.is_ok(),
                message: match &result {
                    Ok(_) => format!("已连接 API：{}", api.base_url()),
                    Err(e) => format!("API 未连接：{e}"),
                },
            });
        });
    }

    // ---------- 列表视图 ----------

    fn visible_len(&self) -> usize {
        if let Some(search) = &self.search {
            return search.songs.len();
        }
        match self.view {
            View::Home => HOME_ENTRIES.len(),
            View::Netease => match self.netease_page {
                NeteasePage::Browse => self.netease_chart.len(),
                NeteasePage::Search => self.netease_playlist_hits.len(),
                NeteasePage::Tracks => self
                    .netease_tracks
                    .as_ref()
                    .map(|t| t.songs.len())
                    .unwrap_or(0),
            },
            View::Indie => match self.indie_page {
                IndiePage::Playlists => self.indie_playlists.len(),
                IndiePage::Tracks => self
                    .indie_tracks
                    .as_ref()
                    .map(|t| t.songs.len())
                    .unwrap_or(0),
            },
            View::StreetVoiceRecommend => self.sv_recommend.len(),
            View::Local => self.local_songs.len(),
            View::Recent => self.history.len(),
        }
    }

    fn ensure_selection(&mut self) {
        let len = self.visible_len();
        if len == 0 {
            self.list_state.select(None);
            return;
        }
        let key = self.list_key();
        let index = self
            .list_state
            .selected()
            .or_else(|| self.selection_memory.get(&key).copied())
            .unwrap_or(0)
            .min(len - 1);
        self.list_state.select(Some(index));
    }

    /// 当前列表在 `selection_memory` 中的键。
    fn list_key(&self) -> ListKey {
        if self.search.is_some() {
            return ListKey::Search;
        }
        match self.view {
            View::Home => ListKey::Home,
            View::Netease => match self.netease_page {
                NeteasePage::Browse => ListKey::NeteaseBrowse,
                NeteasePage::Search => ListKey::NeteaseSearch,
                NeteasePage::Tracks => ListKey::NeteaseTracks(
                    self.netease_tracks
                        .as_ref()
                        .map(|t| t.id.clone())
                        .unwrap_or_default(),
                ),
            },
            View::Indie => match self.indie_page {
                IndiePage::Playlists => ListKey::IndiePlaylists,
                IndiePage::Tracks => ListKey::IndieTracks(
                    self.indie_tracks
                        .as_ref()
                        .map(|t| t.id.clone())
                        .unwrap_or_default(),
                ),
            },
            View::StreetVoiceRecommend => ListKey::StreetVoiceRecommend,
            View::Local => ListKey::Local,
            View::Recent => ListKey::Recent,
        }
    }

    /// 切换列表前保存当前指针，返回 / 重进时恢复。
    fn remember_selection(&mut self) {
        if let Some(index) = self.list_state.selected() {
            self.selection_memory.insert(self.list_key(), index);
        }
    }

    fn restore_selection(&mut self) {
        let len = self.visible_len();
        if len == 0 {
            self.list_state.select(None);
            return;
        }
        let index = self
            .selection_memory
            .get(&self.list_key())
            .copied()
            .unwrap_or(0)
            .min(len - 1);
        self.list_state.select(Some(index));
    }

    fn move_selection(&mut self, delta: i64) {
        let len = self.visible_len();
        if len == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0) as i64;
        let next = (current + delta).clamp(0, len as i64 - 1) as usize;
        self.list_state.select(Some(next));
    }

    fn select_boundary(&mut self, last: bool) {
        let len = self.visible_len();
        if len == 0 {
            return;
        }
        self.list_state.select(Some(if last { len - 1 } else { 0 }));
    }

    /// 当前列表里是否包含正在播放的歌曲，返回它的行号。
    fn visible_song_index(&self, key: &str) -> Option<usize> {
        if let Some(search) = &self.search {
            return search.songs.iter().position(|song| song.key() == key);
        }
        let songs: &[Song] = match self.view {
            View::Home => return None,
            View::Netease => match self.netease_page {
                NeteasePage::Browse => &self.netease_chart,
                NeteasePage::Search => return None,
                NeteasePage::Tracks => self
                    .netease_tracks
                    .as_ref()
                    .map(|tracks| tracks.songs.as_slice())
                    .unwrap_or(&[]),
            },
            View::Indie => match self.indie_page {
                IndiePage::Playlists => return None,
                IndiePage::Tracks => self
                    .indie_tracks
                    .as_ref()
                    .map(|tracks| tracks.songs.as_slice())
                    .unwrap_or(&[]),
            },
            View::StreetVoiceRecommend => &self.sv_recommend,
            View::Local => &self.local_songs,
            View::Recent => &self.history,
        };
        songs.iter().position(|song| song.key() == key)
    }

    /// 跟随当前歌曲：自动切歌后把列表指针移到正在播放的那一行。
    fn reveal_current(&mut self) {
        let Some(key) = self.current.as_ref().map(|song| song.key()) else {
            return;
        };
        if let Some(index) = self.visible_song_index(&key) {
            self.list_state.select(Some(index));
            self.selection_memory.insert(self.list_key(), index);
        }
    }

    fn scroll_lyrics(&mut self, delta: i32) {
        if !self.show_lyrics && !self.lyrics_immersive {
            return;
        }
        self.lyrics_scroll = self.lyrics_scroll.saturating_add(delta);
    }

    fn loading(&self) -> bool {
        if self.search.is_some() {
            return self.search_loading;
        }
        match self.view {
            View::Home => false,
            View::Netease => match self.netease_page {
                NeteasePage::Browse => self.netease_chart_loading,
                _ => self.netease_loading,
            },
            View::Indie => self.indie_loading,
            View::StreetVoiceRecommend => self.sv_recommend_loading,
            View::Local => self.local_loading,
            View::Recent => false,
        }
    }

    fn current_list_title(&self) -> String {
        if let Some(search) = &self.search {
            return format!("搜索 · {}（{}）", search.query, search.songs.len());
        }
        match self.view {
            View::Home => "首页".to_string(),
            View::Netease => match self.netease_page {
                NeteasePage::Browse => format!("网易原创（{}）", self.netease_chart.len()),
                NeteasePage::Search => format!(
                    "网易云歌单搜索 · {}（{}）",
                    self.netease_query,
                    self.netease_playlist_hits.len()
                ),
                NeteasePage::Tracks => self
                    .netease_tracks
                    .as_ref()
                    .map(|t| format!("{}（{}）", t.title, t.songs.len()))
                    .unwrap_or_else(|| "网易云歌单".into()),
            },
            View::Indie => match self.indie_page {
                IndiePage::Playlists => format!("雀乐推荐（{}）", self.indie_playlists.len()),
                IndiePage::Tracks => self
                    .indie_tracks
                    .as_ref()
                    .map(|t| format!("{}（{}）", t.title, t.songs.len()))
                    .unwrap_or_else(|| "雀乐歌单".into()),
            },
            View::StreetVoiceRecommend => format!("街声推荐（{}）", self.sv_recommend.len()),
            View::Local => format!("本地列表（{}）", self.local_songs.len()),
            View::Recent => format!("最近播放（{}）", self.history.len()),
        }
    }

    fn empty_message(&self) -> &'static str {
        if self.search.is_some() {
            return "没有找到歌曲 · Esc 关闭搜索";
        }
        match self.view {
            View::Home => "栏目加载中…",
            View::Netease => match self.netease_page {
                NeteasePage::Browse => "榜单为空 · R 刷新",
                NeteasePage::Search => "没有找到歌单 · Esc 返回",
                NeteasePage::Tracks => "歌单为空 · Esc 返回",
            },
            View::Indie => match self.indie_page {
                IndiePage::Playlists => "暂无雀乐歌单 · R 刷新",
                IndiePage::Tracks => "歌单为空 · Esc 返回",
            },
            View::StreetVoiceRecommend => "暂无推荐 · R 刷新",
            View::Local => "本地目录为空 · 在配置 [local] dir 指定目录",
            View::Recent => "还没有播放记录",
        }
    }

    fn selected_index(&self) -> Option<usize> {
        self.list_state
            .selected()
            .filter(|i| *i < self.visible_len())
    }

    // ---------- 视图切换与加载 ----------

    /// 回到首页，并尽量把光标停在与刚才栏目对应的入口上。
    fn go_home(&mut self) {
        let previous = self.view;
        self.remember_selection();
        self.search_seq += 1;
        self.search = None;
        self.search_loading = false;
        self.focus = Focus::List;
        self.input.clear();
        self.input_cursor = 0;
        self.error = None;
        self.view = View::Home;
        let index = HOME_ENTRIES
            .iter()
            .position(|view| *view == previous)
            .unwrap_or(0);
        self.selection_memory.insert(ListKey::Home, index);
        self.restore_selection();
        self.ensure_home_data();
    }

    fn ensure_home_data(&mut self) {
        if !self.netease_chart_loaded && !self.netease_chart_loading {
            self.load_netease_chart();
        }
        if !self.sv_recommend_loaded && !self.sv_recommend_loading {
            self.load_streetvoice();
        }
        if !self.indie_loaded && !self.indie_loading {
            self.load_indie();
        }
        if !self.local_loaded && !self.local_loading {
            self.scan_local();
        }
        self.history = history::recent(&self.paths.history_file, 50);
    }

    fn ensure_view_data(&mut self) {
        match self.view {
            View::Home => self.ensure_home_data(),
            View::Netease if self.netease_page == NeteasePage::Browse => {
                if !self.netease_chart_loaded && !self.netease_chart_loading {
                    self.load_netease_chart();
                }
            }
            View::Indie if self.indie_page == IndiePage::Playlists => {
                if !self.indie_loaded && !self.indie_loading {
                    self.load_indie();
                }
            }
            View::StreetVoiceRecommend
                if !self.sv_recommend_loaded && !self.sv_recommend_loading =>
            {
                self.load_streetvoice();
            }
            View::Local if !self.local_loaded && !self.local_loading => self.scan_local(),
            View::Recent => {
                self.history = history::recent(&self.paths.history_file, 50);
            }
            _ => {}
        }
    }

    fn open_home_entry(&mut self, index: usize) {
        let Some(view) = HOME_ENTRIES.get(index).copied() else {
            return;
        };
        self.remember_selection();
        self.search_seq += 1;
        self.search = None;
        self.search_loading = false;
        self.focus = Focus::List;
        self.input.clear();
        self.input_cursor = 0;
        self.error = None;
        self.view = view;
        match view {
            View::Netease => self.netease_page = NeteasePage::Browse,
            View::Indie => self.indie_page = IndiePage::Playlists,
            _ => {}
        }
        self.restore_selection();
        self.ensure_view_data();
    }

    fn load_netease_chart(&mut self) {
        self.netease_chart_seq += 1;
        let seq = self.netease_chart_seq;
        self.netease_chart_loading = true;
        self.error = None;
        let api = self.api.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = api.original_chart(30).await.map_err(|e| e.to_string());
            let _ = tx.send(Event::NeteaseChart { seq, result });
        });
    }

    fn search_netease_playlists(&mut self, query: &str) {
        let query = query.trim().to_string();
        if query.is_empty() {
            return;
        }
        self.remember_selection();
        self.search_seq += 1;
        self.search = None;
        self.search_loading = false;
        self.netease_seq += 1;
        let seq = self.netease_seq;
        self.netease_loading = true;
        self.view = View::Netease;
        self.netease_page = NeteasePage::Search;
        self.netease_back_to_search = false;
        if self.netease_query != query {
            self.selection_memory.insert(ListKey::NeteaseSearch, 0);
        }
        self.netease_query = query.clone();
        self.netease_playlist_hits.clear();
        self.restore_selection();
        self.error = None;
        let api = self.api.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = api
                .search_playlists(&query, 50)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Event::NeteasePlaylistHits { seq, query, result });
        });
    }

    fn open_netease_playlist(&mut self, id: String, title: String) {
        self.remember_selection();
        self.netease_seq += 1;
        let seq = self.netease_seq;
        self.netease_back_to_search = self.view == View::Netease
            && matches!(self.netease_page, NeteasePage::Browse | NeteasePage::Search)
            && self.netease_page == NeteasePage::Search;
        self.view = View::Netease;
        self.netease_loading = true;
        self.netease_page = NeteasePage::Tracks;
        self.netease_tracks = Some(Opened {
            id: id.clone(),
            title: title.clone(),
            songs: Vec::new(),
        });
        self.restore_selection();
        self.error = None;
        let api = self.api.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = api
                .playlist_tracks(&id, 1000, 0)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Event::NeteaseTracks {
                seq,
                id,
                title,
                result,
            });
        });
    }

    fn load_indie(&mut self) {
        self.indie_seq += 1;
        let seq = self.indie_seq;
        self.indie_loading = true;
        self.error = None;
        let api = self.api.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = api.indie_catalog().await.map_err(|e| e.to_string());
            let _ = tx.send(Event::IndieCatalog { seq, result });
        });
    }

    fn open_indie_playlist(&mut self, index: usize) {
        let Some(playlist) = self.indie_playlists.get(index).cloned() else {
            return;
        };
        self.remember_selection();
        self.indie_tracks = Some(Opened {
            id: playlist.id.clone(),
            title: playlist.name.clone(),
            songs: playlist.songs.clone(),
        });
        self.indie_page = IndiePage::Tracks;
        self.restore_selection();
    }

    fn load_streetvoice(&mut self) {
        self.sv_recommend_seq += 1;
        let seq = self.sv_recommend_seq;
        self.sv_recommend_loading = true;
        self.error = None;
        let api = self.api.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = api.streetvoice_recommend().await.map_err(|e| e.to_string());
            let _ = tx.send(Event::StreetVoiceRecommend { seq, result });
        });
    }

    fn scan_local(&mut self) {
        self.local_seq += 1;
        let seq = self.local_seq;
        self.local_loading = true;
        self.error = None;
        let dir = self.cfg.local.resolved_dir();
        let output_dir = self.cfg.local.resolved_output_dir();
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            let result = scan_local_dir(&dir, &output_dir);
            let _ = tx.send(Event::LocalSongs { seq, result });
        });
    }

    fn refresh_current(&mut self) {
        self.error = None;
        match self.view {
            View::Home => {
                self.load_netease_chart();
                self.load_streetvoice();
                self.load_indie();
                self.scan_local();
                self.history = history::recent(&self.paths.history_file, 50);
                self.toast("正在刷新首页…");
            }
            View::Netease => match self.netease_page {
                NeteasePage::Tracks => {
                    if let Some(tracks) = self.netease_tracks.clone() {
                        self.open_netease_playlist(tracks.id, tracks.title);
                    } else {
                        self.netease_page = NeteasePage::Browse;
                        self.load_netease_chart();
                    }
                }
                NeteasePage::Search => {
                    let query = self.netease_query.clone();
                    self.search_netease_playlists(&query);
                }
                NeteasePage::Browse => self.load_netease_chart(),
            },
            View::Indie => {
                self.indie_page = IndiePage::Playlists;
                self.indie_tracks = None;
                self.load_indie();
            }
            View::StreetVoiceRecommend => self.load_streetvoice(),
            View::Local => self.scan_local(),
            View::Recent => {
                self.history = history::recent(&self.paths.history_file, 50);
                self.toast("已刷新最近播放");
            }
        }
    }

    // ---------- 输入与搜索 ----------

    fn submit_input(&mut self) {
        let raw = self.input.trim().to_string();
        if raw.is_empty() {
            self.focus = Focus::List;
            return;
        }
        let mode = self.input_mode;
        self.focus = Focus::List;
        match mode {
            InputMode::Search => self.search_songs_now(&raw),
            InputMode::PlaylistSearch => self.search_netease_playlists(&raw),
            InputMode::Open => self.submit_raw(&raw),
        }
    }

    fn submit_raw(&mut self, raw: &str) {
        let text = raw.trim();
        if text.is_empty() {
            return;
        }
        match input::classify(text) {
            Classified::M3u(path) => self.open_m3u(path),
            Classified::Local(path) => self.open_local(path),
            Classified::Playlist(id) => {
                self.open_netease_playlist(id.clone(), format!("歌单 {id}"));
            }
            Classified::Song(id) => self.open_song_id(id),
            Classified::Query(query) => self.search_songs_now(&query),
        }
        self.focus = Focus::List;
    }

    fn search_songs_now(&mut self, query: &str) {
        let query = query.trim().to_string();
        if query.is_empty() {
            return;
        }
        let same_query = self.search.as_ref().is_some_and(|s| s.query == query);
        self.remember_selection();
        if !same_query {
            self.selection_memory.insert(ListKey::Search, 0);
        }
        self.search_seq += 1;
        let seq = self.search_seq;
        self.search_loading = true;
        self.error = None;
        self.search = Some(SearchResults {
            query: query.clone(),
            songs: Vec::new(),
        });
        self.restore_selection();
        let api = self.api.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = api
                .search_songs(&query, 50, 0)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Event::SearchSongs {
                seq,
                query,
                autoplay: false,
                result,
            });
        });
    }

    fn open_song_id(&mut self, id: String) {
        self.open_seq += 1;
        let seq = self.open_seq;
        self.error = None;
        let api = self.api.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = match api.song_detail(std::slice::from_ref(&id)).await {
                Ok(songs) if !songs.is_empty() => Ok(songs),
                Ok(_) => api
                    .playlist_tracks(&id, 1000, 0)
                    .await
                    .map_err(|e| e.to_string()),
                Err(e) => api
                    .playlist_tracks(&id, 1000, 0)
                    .await
                    .map_err(|_| e.to_string()),
            };
            let _ = tx.send(Event::OpenSongs {
                seq,
                title: "链接打开".into(),
                result,
            });
        });
    }

    fn open_local(&mut self, path: PathBuf) {
        if !path.exists() {
            self.toast(format!("文件不存在：{}", path.display()));
            return;
        }
        let song = local_song(&path, None, 0);
        self.adopt_queue(format!("本地 · {}", song.name), vec![song]);
    }

    fn open_m3u(&mut self, path: PathBuf) {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                self.toast(format!("读取歌单失败：{e}"));
                return;
            }
        };
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        let entries = m3u::parse(&text, base);
        if entries.is_empty() {
            self.toast("歌单为空，或没有可识别的条目");
            return;
        }

        let title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("M3U")
            .to_string();
        let mut songs = Vec::with_capacity(entries.len());
        let mut online: Vec<(usize, String)> = Vec::new();
        for entry in entries {
            match entry {
                m3u::M3uEntry::Local {
                    path,
                    title,
                    duration_ms,
                } => songs.push(local_song(&path, title, duration_ms)),
                m3u::M3uEntry::Online { id, title } => {
                    let index = songs.len();
                    songs.push(Song {
                        name: title.unwrap_or_else(|| format!("网易云 {id}")),
                        id: id.clone(),
                        source: SourceKind::Netease,
                        ..Song::default()
                    });
                    online.push((index, id));
                }
            }
        }

        if online.is_empty() {
            self.adopt_queue(title, songs);
            return;
        }

        self.open_seq += 1;
        let seq = self.open_seq;
        self.error = None;
        let api = self.api.clone();
        let tx = self.tx.clone();
        let ids: Vec<String> = online.iter().map(|(_, id)| id.clone()).collect();
        tokio::spawn(async move {
            let result = api
                .song_detail(&ids)
                .await
                .map(|details| {
                    let mut map: HashMap<String, Song> = details
                        .into_iter()
                        .map(|song| (song.id.clone(), song))
                        .collect();
                    for (index, id) in online {
                        if let Some(song) = map.remove(&id) {
                            songs[index] = song;
                        }
                    }
                    songs
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(Event::OpenSongs { seq, title, result });
        });
    }

    fn adopt_queue(&mut self, title: String, songs: Vec<Song>) {
        if songs.is_empty() {
            self.toast("列表为空");
            return;
        }
        let count = songs.len();
        self.queue = songs;
        self.remember_selection();
        self.search_seq += 1;
        self.search = None;
        self.search_loading = false;
        self.focus = Focus::List;
        self.input.clear();
        self.input_cursor = 0;
        self.input_mode = InputMode::Search;
        self.error = None;
        self.restore_selection();
        self.toast(format!("已打开：{title}（{count} 首）"));
        self.play_at(0, 0, true);
    }

    fn play_list(&mut self, songs: Vec<Song>, index: usize) {
        if songs.is_empty() || index >= songs.len() {
            return;
        }
        self.queue = songs;
        self.play_at(index, 0, true);
    }

    fn activate_selected(&mut self) {
        let Some(index) = self.selected_index() else {
            return;
        };
        if self.search.is_some() {
            let songs = self
                .search
                .as_ref()
                .map(|s| s.songs.clone())
                .unwrap_or_default();
            self.play_list(songs, index);
            return;
        }
        match self.view {
            View::Home => self.open_home_entry(index),
            View::Netease => match self.netease_page {
                NeteasePage::Browse => self.play_list(self.netease_chart.clone(), index),
                NeteasePage::Search => {
                    if let Some(playlist) = self.netease_playlist_hits.get(index).cloned() {
                        self.open_netease_playlist(playlist.id, playlist.name);
                    }
                }
                NeteasePage::Tracks => {
                    if let Some(tracks) = self.netease_tracks.clone() {
                        self.play_list(tracks.songs, index);
                    }
                }
            },
            View::Indie => match self.indie_page {
                IndiePage::Playlists => self.open_indie_playlist(index),
                IndiePage::Tracks => {
                    if let Some(tracks) = self.indie_tracks.clone() {
                        self.play_list(tracks.songs, index);
                    }
                }
            },
            View::StreetVoiceRecommend => self.play_list(self.sv_recommend.clone(), index),
            View::Local => self.play_list(self.local_songs.clone(), index),
            View::Recent => self.play_list(self.history.clone(), index),
        }
    }

    // ---------- 播放 ----------

    fn play_at(&mut self, index: usize, start_ms: u64, autoplay: bool) {
        let Some(song) = self.queue.get(index).cloned() else {
            return;
        };
        self.cursor = Some(index);
        self.current = Some(song.clone());
        self.loaded = false;
        self.playing = false;
        self.loading_play = true;
        // 立刻停掉上一首，避免解析新音源时旧曲还在响、旧进度也继续上报。
        self.engine.send(Cmd::Stop);
        self.pos_ms = start_ms;
        self.dur_ms = song.duration_ms;
        self.next_start_ms = start_ms;
        self.pending_autoplay = autoplay;
        self.cache_progress = None;
        self.play_seq += 1;
        let seq = self.play_seq;

        self.load_lyrics(&song);
        self.load_cover(&song);
        if song.source == SourceKind::Local {
            self.start_local(&song, start_ms, autoplay);
        } else {
            self.spawn_resolve(seq, song);
        }
        self.reveal_current();
        self.sync_mpris(true);
    }

    fn start_local(&mut self, song: &Song, start_ms: u64, autoplay: bool) {
        let path = PathBuf::from(&song.id);
        if !path.exists() {
            // 转换副本被删或原目录移动时，直接跳过，避免卡在坏文件上。
            self.skip_unplayable("本地文件不存在，可能已被移动");
            return;
        }
        let duration = if song.duration_ms > 0 {
            song.duration_ms
        } else {
            musicyi_library::read_meta(&path)
                .map(|m| m.duration_ms)
                .unwrap_or(0)
        };
        self.dur_ms = duration;
        self.engine.send(Cmd::Load {
            path,
            duration_ms: duration,
            start_ms,
            autoplay,
        });
        self.loading_play = false;
        self.loaded = true;
        self.playing = autoplay;
        if autoplay {
            self.record_history(song);
        }
        self.sync_mpris(true);
    }

    fn spawn_resolve(&mut self, seq: u64, song: Song) {
        let resolver = self.resolver.clone();
        let tx = self.tx.clone();
        let http = self.http.clone();
        let cache = self.cache.clone();
        let quality = self.quality;
        tokio::spawn(async move {
            let source = match resolver.resolve(&song, quality).await {
                Ok(source) => source,
                Err(e) => {
                    let _ = tx.send(Event::PlayError {
                        seq,
                        message: e.to_string(),
                    });
                    return;
                }
            };
            let mut song = song;
            if let Some(duration) = source.duration_ms.filter(|d| *d > 0) {
                song.duration_ms = duration;
            }
            if let Some(album) = source.album.clone().filter(|a| !a.trim().is_empty()) {
                song.album = album;
            }
            let _ = tx.send(Event::Resolved {
                seq,
                song: song.clone(),
                url: source.url.clone(),
                provider: source.provider.clone(),
                format: source.format.clone(),
                br: source.br,
                preview: source.preview,
                duration_ms: source.duration_ms,
            });

            let key = format!("{}:{}:{}", song.key(), quality.level(), source.br);
            let tx_progress = tx.clone();
            let mut last_pct = i32::MIN;
            let result = cache
                .fetch(
                    &http,
                    &source.url,
                    &key,
                    &source.format,
                    move |done, total| {
                        let pct = total
                            .filter(|t| *t > 0)
                            .map(|t| (done.saturating_mul(100) / t) as i32)
                            .unwrap_or(-1);
                        if pct != last_pct {
                            last_pct = pct;
                            let _ = tx_progress.send(Event::CacheProgress { seq, done, total });
                        }
                    },
                )
                .await;
            match result {
                Ok(path) => {
                    let _ = tx.send(Event::CacheReady {
                        seq,
                        song,
                        path,
                        provider: source.provider,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Event::PlayError {
                        seq,
                        message: e.to_string(),
                    });
                }
            }
        });
    }

    /// 播放失败时自动跳到下一首；连续失败达到队列长度或 8 首后停止。
    fn skip_unplayable(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.failed_streak += 1;
        self.loading_play = false;
        self.loaded = false;
        self.playing = false;
        self.cache_progress = None;
        let limit = self.queue.len().clamp(1, 8) as u32;
        if self.queue.is_empty() || self.failed_streak >= limit {
            self.failed_streak = 0;
            self.toast(format!("连续播放失败，已停止：{message}"));
            self.sync_mpris(true);
            return;
        }
        self.toast(format!("跳过无法播放的歌曲：{message}"));
        self.next_song(true);
    }

    /// 解析出时长 / 专辑后，把队列和各个列表里的同一首歌一起更新。
    fn patch_song(&mut self, key: &str, duration: Option<u64>, album: Option<&str>) {
        fn apply(song: &mut Song, key: &str, duration: Option<u64>, album: Option<&str>) {
            if song.key() != key {
                return;
            }
            if let Some(duration) = duration {
                song.duration_ms = duration;
            }
            if let Some(album) = album.filter(|a| !a.trim().is_empty()) {
                song.album = album.to_string();
            }
        }
        let patch = |song: &mut Song| apply(song, key, duration, album);
        if let Some(song) = &mut self.current {
            patch(song);
        }
        for song in &mut self.queue {
            patch(song);
        }
        for song in &mut self.netease_chart {
            patch(song);
        }
        for song in &mut self.sv_recommend {
            patch(song);
        }
        if let Some(opened) = &mut self.netease_tracks {
            for song in &mut opened.songs {
                patch(song);
            }
        }
        if let Some(opened) = &mut self.indie_tracks {
            for song in &mut opened.songs {
                patch(song);
            }
        }
    }

    fn next_song(&mut self, auto: bool) {
        if self.queue.is_empty() {
            return;
        }
        let index = self.cursor.unwrap_or(0).min(self.queue.len() - 1);
        let next = match self.mode {
            PlayMode::Single if auto => Some(index),
            PlayMode::Single => Some((index + 1) % self.queue.len()),
            PlayMode::ListLoop => Some((index + 1) % self.queue.len()),
            PlayMode::Sequential => {
                if index + 1 < self.queue.len() {
                    Some(index + 1)
                } else {
                    None
                }
            }
            PlayMode::Shuffle => {
                if self.queue.len() == 1 {
                    Some(0)
                } else {
                    let mut next = index;
                    while next == index {
                        next = rand::random_range(0..self.queue.len());
                    }
                    Some(next)
                }
            }
        };
        match next {
            Some(index) => self.play_at(index, 0, true),
            None => {
                self.playing = false;
                self.loaded = false;
                self.toast("队列播放完毕");
                self.sync_mpris(true);
            }
        }
    }

    fn previous_song(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let index = self.cursor.unwrap_or(0).min(self.queue.len() - 1);
        let previous = if index == 0 {
            self.queue.len() - 1
        } else {
            index - 1
        };
        self.play_at(previous, 0, true);
    }

    fn toggle_play(&mut self) {
        if self.loading_play {
            self.toast("正在加载，请稍候…");
            return;
        }
        match self.current.clone() {
            None => {
                if self.queue.is_empty() {
                    self.toast("先搜索或打开一首歌");
                } else {
                    self.play_at(0, 0, true);
                }
            }
            Some(_) if !self.loaded => {
                let index = self.cursor.unwrap_or(0);
                let start = self.pos_ms;
                self.play_at(index, start, true);
            }
            Some(_) => {
                self.playing = !self.playing;
                self.engine.send(Cmd::Toggle);
                self.sync_mpris(true);
            }
        }
    }

    fn seek_to(&mut self, ms: u64) {
        let target = if self.dur_ms > 0 {
            ms.min(self.dur_ms)
        } else {
            ms
        };
        self.pos_ms = target;
        if self.loaded {
            self.engine.send(Cmd::Seek(Duration::from_millis(target)));
            if let Some(tx) = &self.mpris_cmd {
                let _ = tx.send(MprisCmd::Seeked(target));
            }
        }
    }

    fn seek_by(&mut self, delta_ms: i64) {
        let pos = (self.pos_ms as i64 + delta_ms).max(0) as u64;
        self.seek_to(pos);
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.5);
        self.cfg.player.volume = self.volume;
        self.engine.send(Cmd::Volume(self.volume));
        self.toast(format!("音量 {:.0}%", self.volume * 100.0));
        self.sync_mpris(true);
    }

    fn cycle_mode(&mut self) {
        self.set_mode(self.mode.cycle());
    }

    fn set_mode(&mut self, mode: PlayMode) {
        self.mode = mode;
        self.cfg.player.mode = mode;
        self.toast(format!("播放模式：{}", mode.label()));
        self.sync_mpris(true);
    }

    fn set_shuffle(&mut self, shuffle: bool) {
        let mode = if shuffle {
            PlayMode::Shuffle
        } else if self.mode == PlayMode::Shuffle {
            PlayMode::ListLoop
        } else {
            self.mode
        };
        self.set_mode(mode);
    }

    fn record_history(&mut self, song: &Song) {
        if let Err(e) = history::append(&self.paths.history_file, song) {
            tracing::warn!("写入历史失败: {e}");
        }
        self.history = history::recent(&self.paths.history_file, 50);
    }

    /// 仅当列表展示的是“最近播放”时允许删除。
    fn remove_history_selected(&mut self) {
        if self.view != View::Recent || self.search.is_some() {
            self.toast("只能删除最近播放中的记录");
            return;
        }
        let Some(index) = self.selected_index() else {
            return;
        };
        let Some(song) = self.history.get(index).cloned() else {
            return;
        };
        match history::remove(&self.paths.history_file, &song.key()) {
            Ok(0) => self.toast("记录已不存在"),
            Ok(_) => {
                self.history = history::recent(&self.paths.history_file, 50);
                self.ensure_selection();
                self.toast(format!("已从最近播放移除：{}", song.name));
            }
            Err(e) => self.toast(format!("删除记录失败：{e}")),
        }
    }

    // ---------- 歌词 ----------

    fn load_lyrics(&mut self, song: &Song) {
        let key = song.key();
        self.lyrics = Lyrics::default();
        self.lyrics_key = Some(key.clone());
        self.lyrics_scroll = 0;
        match song.source {
            SourceKind::Local => {
                if let Some(text) = musicyi_library::read_lyrics(Path::new(&song.id)) {
                    self.lyrics = Lyrics::parse(&text, "");
                }
            }
            SourceKind::Indie => {
                let Some(url) = song.lyric_url.clone() else {
                    return;
                };
                let http = self.http.clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    if let Ok(resp) = http.get(url).send().await
                        && let Ok(text) = resp.text().await
                    {
                        let _ = tx.send(Event::Lyric {
                            key,
                            raw: RawLyric {
                                lrc: text,
                                tlyric: String::new(),
                            },
                        });
                    }
                });
            }
            SourceKind::StreetVoice => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                let id = song.id.clone();
                tokio::spawn(async move {
                    if let Ok(detail) = api.streetvoice_detail(&id).await
                        && !detail.lyrics.trim().is_empty()
                    {
                        let _ = tx.send(Event::Lyric {
                            key,
                            raw: RawLyric {
                                lrc: detail.lyrics,
                                tlyric: String::new(),
                            },
                        });
                    }
                });
            }
            _ => {
                let api = self.api.clone();
                let tx = self.tx.clone();
                let id = song.id.clone();
                tokio::spawn(async move {
                    if let Ok(raw) = api.lyric(&id).await {
                        let _ = tx.send(Event::Lyric { key, raw });
                    }
                });
            }
        }
    }

    // ---------- 封面 ----------

    fn load_cover(&mut self, song: &Song) {
        let key = song.key();
        if self.cover.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        if let Some(art) = self.cover_cache.get(&key) {
            self.cover = Some((key, art.clone()));
            return;
        }
        self.cover = None;
        self.cover_seq += 1;
        let tx = self.tx.clone();
        let http = self.http.clone();
        let cover_url = song.cover_url.clone();
        let local_path = (song.source == SourceKind::Local).then(|| PathBuf::from(&song.id));
        tokio::spawn(async move {
            let bytes = if let Some(path) = local_path {
                tokio::task::spawn_blocking(move || musicyi_library::read_cover(&path))
                    .await
                    .ok()
                    .flatten()
            } else if let Some(url) = cover_url.filter(|u| !u.trim().is_empty()) {
                match http.get(&url).send().await {
                    Ok(resp) => resp.bytes().await.ok().map(|b| b.to_vec()),
                    Err(_) => None,
                }
            } else {
                None
            };
            let Some(bytes) = bytes else {
                let _ = tx.send(Event::Cover {
                    key,
                    result: Err("没有封面".into()),
                });
                return;
            };
            let result = match tokio::task::spawn_blocking(move || decode_cover(&bytes)).await {
                Ok(Some(art)) => Ok(art),
                Ok(None) => Err("封面解码失败".into()),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(Event::Cover { key, result });
        });
    }

    // ---------- 本地标签补全 ----------

    fn tag_selected(&mut self) {
        if self.view != View::Local || self.search.is_some() {
            self.toast("只能在本地列表里补全标签");
            return;
        }
        if self.tagging {
            self.toast("已有补全任务在进行");
            return;
        }
        let Some(index) = self.selected_index() else {
            return;
        };
        let Some(song) = self.local_songs.get(index).cloned() else {
            return;
        };
        let origin = local_origin(&song);
        if !origin.exists() {
            self.toast("本地文件不存在");
            return;
        }
        let source_dir = self.cfg.local.resolved_dir();
        let output_dir = self.cfg.local.resolved_output_dir();
        if same_dir(&source_dir, &output_dir) {
            self.toast("转换目录不能与源目录相同，请修改配置 [local] output_dir");
            return;
        }

        self.tag_seq += 1;
        let seq = self.tag_seq;
        self.tagging = true;
        self.toast = None;
        self.tag_progress = Some((0, 1, song.name.clone()));
        let api = self.api.clone();
        let http = self.http.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let old_id = song.id.clone();
            let (hint_artist, hint_title) = musicyi_library::filename_hint(&origin);
            let title = hint_title
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| song.name.clone());
            let artist = hint_artist
                .clone()
                .or_else(|| song.artists.first().cloned())
                .unwrap_or_default();
            let query = if artist.trim().is_empty() {
                title.clone()
            } else {
                format!("{title} {}", artist.trim())
            };

            let updated = match api.search_songs(&query, 15, 0).await {
                Ok(candidates) => {
                    match pick_best_match(
                        &song,
                        hint_artist.as_deref(),
                        hint_title.as_deref(),
                        &candidates,
                    ) {
                        Some(matched) => {
                            match convert_and_tag(
                                &api,
                                &http,
                                &song,
                                &matched,
                                &source_dir,
                                &output_dir,
                            )
                            .await
                            {
                                Ok(update) => Some(update),
                                Err(e) => {
                                    let _ = tx.send(Event::TagResult {
                                        seq,
                                        old_id,
                                        updated: None,
                                        message: e,
                                    });
                                    return;
                                }
                            }
                        }
                        None => None,
                    }
                }
                Err(e) => {
                    let _ = tx.send(Event::TagResult {
                        seq,
                        old_id,
                        updated: None,
                        message: format!("搜索失败：{e}"),
                    });
                    return;
                }
            };

            let message = match &updated {
                Some(update) => format!(
                    "已生成转换副本：{} — {}",
                    update.title,
                    update.artists.join(" / ")
                ),
                None => format!("没有找到匹配（{query}）"),
            };
            let _ = tx.send(Event::TagResult {
                seq,
                old_id,
                updated,
                message,
            });
        });
    }

    fn tag_all_missing(&mut self) {
        if self.view != View::Local || self.search.is_some() {
            self.toast("只能在本地列表里补全标签");
            return;
        }
        if self.tagging {
            self.toast("已有补全任务在进行");
            return;
        }
        if self.local_loading {
            self.toast("正在扫描本地目录，请稍候…");
            return;
        }
        let pending: Vec<Song> = self
            .local_songs
            .iter()
            .filter(|song| self.local_needs_tag.contains(&song.id))
            .cloned()
            .collect();
        if pending.is_empty() {
            self.toast("本地标签都已完整");
            return;
        }
        let source_dir = self.cfg.local.resolved_dir();
        let output_dir = self.cfg.local.resolved_output_dir();
        if same_dir(&source_dir, &output_dir) {
            self.toast("转换目录不能与源目录相同，请修改配置 [local] output_dir");
            return;
        }

        self.tag_seq += 1;
        let seq = self.tag_seq;
        let total = pending.len() as u64;
        self.tagging = true;
        self.toast = None;
        self.tag_progress = Some((0, total, String::new()));
        let api = self.api.clone();
        let http = self.http.clone();
        let tx = self.tx.clone();
        let delay = self.cfg.local.tag_delay_ms.min(2000);
        tokio::spawn(async move {
            let mut fixed = 0u64;
            let mut failed = 0u64;
            let mut consecutive = 0u32;
            for (index, song) in pending.iter().enumerate() {
                let origin = local_origin(song);
                let (hint_artist, hint_title) = musicyi_library::filename_hint(&origin);
                let title = hint_title
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| song.name.clone());
                let artist = hint_artist
                    .clone()
                    .or_else(|| song.artists.first().cloned())
                    .unwrap_or_default();
                let query = if artist.trim().is_empty() {
                    title.clone()
                } else {
                    format!("{title} {}", artist.trim())
                };

                let mut wrote = false;
                match api.search_songs(&query, 15, 0).await {
                    Ok(candidates) => {
                        consecutive = 0;
                        if let Some(matched) = pick_best_match(
                            song,
                            hint_artist.as_deref(),
                            hint_title.as_deref(),
                            &candidates,
                        ) && convert_and_tag(
                            &api,
                            &http,
                            song,
                            &matched,
                            &source_dir,
                            &output_dir,
                        )
                        .await
                        .is_ok()
                        {
                            fixed += 1;
                            wrote = true;
                        }
                    }
                    Err(_) => {
                        consecutive += 1;
                    }
                }
                if !wrote {
                    failed += 1;
                }
                let _ = tx.send(Event::TagProgress {
                    seq,
                    done: (index + 1) as u64,
                    total,
                    name: song.name.clone(),
                });
                if consecutive >= 6 {
                    failed += pending.len().saturating_sub(index + 1) as u64;
                    break;
                }
                if delay > 0 {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
            }
            let _ = tx.send(Event::TagDone { seq, fixed, failed });
        });
    }

    // ---------- 下载 ----------

    fn download_current(&mut self) {
        let Some(song) = self.current.clone() else {
            self.toast("当前没有歌曲");
            return;
        };
        if !song.source.is_online() {
            self.toast("本地文件无需下载");
            return;
        }
        if self.downloading.is_some() {
            self.toast("已有下载任务进行中");
            return;
        }

        self.download_seq += 1;
        let seq = self.download_seq;
        self.downloading = Some((seq, song.name.clone()));
        self.download_progress = None;

        let resolver = self.resolver.clone();
        let tx = self.tx.clone();
        let http = self.http.clone();
        let dir = self.cfg.download.resolved_dir();
        let quality = self.quality;
        tokio::spawn(async move {
            let source = match resolver.resolve(&song, quality).await {
                Ok(source) => source,
                Err(e) => {
                    let _ = tx.send(Event::DownloadFailed {
                        seq,
                        message: e.to_string(),
                    });
                    return;
                }
            };
            let artist = musicyi_library::sanitize(&song.artist_text());
            let title = musicyi_library::sanitize(&song.name);
            let ext: String = source
                .format
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            let ext = if ext.is_empty() {
                "mp3".to_string()
            } else {
                ext
            };
            let dest = dir.join(format!("{artist} - {title}.{ext}"));
            let tx_progress = tx.clone();
            let mut last_pct = i32::MIN;
            let result =
                musicyi_library::download_to(&http, &source.url, &dest, move |done, total| {
                    let pct = total
                        .filter(|t| *t > 0)
                        .map(|t| (done.saturating_mul(100) / t) as i32)
                        .unwrap_or(-1);
                    if pct != last_pct {
                        last_pct = pct;
                        let _ = tx_progress.send(Event::DownloadProgress { seq, done, total });
                    }
                })
                .await;
            match result {
                Ok(outcome) => {
                    let _ = tx.send(Event::DownloadDone {
                        seq,
                        path: outcome.path,
                        bytes: outcome.bytes,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Event::DownloadFailed {
                        seq,
                        message: e.to_string(),
                    });
                }
            }
        });
    }

    // ---------- 事件 ----------

    pub fn handle_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => self.on_key(key),
            Event::Resize => {}
            Event::SearchSongs {
                seq,
                query,
                autoplay,
                result,
            } => {
                if seq != self.search_seq {
                    return;
                }
                self.search_loading = false;
                match result {
                    Ok(songs) => {
                        if autoplay && !songs.is_empty() {
                            self.search = Some(SearchResults {
                                query: query.clone(),
                                songs: songs.clone(),
                            });
                            self.queue = songs;
                            self.play_at(0, 0, true);
                            self.list_state.select(Some(0));
                        } else {
                            let empty = songs.is_empty();
                            self.search = Some(SearchResults { query, songs });
                            self.list_state.select(Some(0));
                            self.error = None;
                            if empty {
                                self.toast("没有找到歌曲");
                            }
                        }
                    }
                    Err(e) => {
                        self.error = Some(e.clone());
                        self.toast(format!("搜索失败：{e}"));
                    }
                }
                self.ensure_selection();
            }
            Event::OpenSongs { seq, title, result } => {
                if seq != self.open_seq {
                    return;
                }
                match result {
                    Ok(songs) if !songs.is_empty() => self.adopt_queue(title, songs),
                    Ok(_) => self.toast("没有找到可播放的歌曲"),
                    Err(e) => self.toast(format!("打开失败：{e}")),
                }
            }
            Event::NeteaseChart { seq, result } => {
                if seq != self.netease_chart_seq {
                    return;
                }
                self.netease_chart_loading = false;
                match result {
                    Ok(songs) => {
                        self.netease_chart = songs;
                        self.netease_chart_loaded = true;
                        self.error = None;
                    }
                    Err(e) => {
                        self.error = Some(e.clone());
                        self.toast(format!("网易原创榜加载失败：{e}"));
                    }
                }
                self.ensure_selection();
            }
            Event::NeteasePlaylistHits { seq, query, result } => {
                if seq != self.netease_seq {
                    return;
                }
                self.netease_loading = false;
                match result {
                    Ok(playlists) => {
                        let empty = playlists.is_empty();
                        self.netease_playlist_hits = playlists;
                        self.netease_query = query;
                        self.error = None;
                        if empty {
                            self.toast("没有找到歌单");
                        }
                    }
                    Err(e) => {
                        self.error = Some(e.clone());
                        self.toast(format!("搜索歌单失败：{e}"));
                    }
                }
                self.ensure_selection();
            }
            Event::NeteaseTracks {
                seq,
                id,
                title,
                result,
            } => {
                if seq != self.netease_seq {
                    return;
                }
                self.netease_loading = false;
                match result {
                    Ok(songs) => {
                        let empty = songs.is_empty();
                        self.netease_tracks = Some(Opened { id, title, songs });
                        self.error = None;
                        if empty {
                            self.toast("歌单为空");
                        }
                    }
                    Err(e) => {
                        self.error = Some(e.clone());
                        self.toast(format!("打开歌单失败：{e}"));
                    }
                }
                self.ensure_selection();
            }
            Event::IndieCatalog { seq, result } => {
                if seq != self.indie_seq {
                    return;
                }
                self.indie_loading = false;
                match result {
                    Ok(playlists) => {
                        self.indie_playlists = playlists;
                        self.indie_loaded = true;
                        self.error = None;
                    }
                    Err(e) => {
                        self.error = Some(e.clone());
                        self.toast(format!("雀乐加载失败：{e}"));
                    }
                }
                self.ensure_selection();
            }
            Event::StreetVoiceRecommend { seq, result } => {
                if seq != self.sv_recommend_seq {
                    return;
                }
                self.sv_recommend_loading = false;
                match result {
                    Ok(songs) => {
                        self.sv_recommend = songs;
                        self.sv_recommend_loaded = true;
                        self.error = None;
                    }
                    Err(e) => {
                        self.error = Some(e.clone());
                        self.toast(format!("街声推荐加载失败：{e}"));
                    }
                }
                self.ensure_selection();
            }
            Event::LocalSongs { seq, result } => {
                if seq != self.local_seq {
                    return;
                }
                self.local_loading = false;
                match result {
                    Ok(scan) => {
                        self.local_songs = scan.songs;
                        self.local_needs_tag = scan.needs_tag.into_iter().collect();
                        self.local_loaded = true;
                        self.error = None;
                    }
                    Err(e) => {
                        self.error = Some(e.clone());
                        self.toast(format!("本地扫描失败：{e}"));
                    }
                }
                self.ensure_selection();
            }
            Event::TagResult {
                seq,
                old_id,
                updated,
                message,
            } => {
                if seq != self.tag_seq {
                    return;
                }
                self.tagging = false;
                self.tag_progress = None;
                if let Some(update) = updated {
                    self.local_needs_tag.remove(&old_id);
                    if let Some(song) = self.local_songs.iter_mut().find(|s| s.id == old_id) {
                        song.id = update.id.clone();
                        song.origin = update.origin.clone();
                        song.name = update.title.clone();
                        song.artists = update.artists.clone();
                        song.album = update.album.clone();
                    }
                    if let Some(current) = &mut self.current
                        && current.id == old_id
                    {
                        current.id = update.id.clone();
                        current.origin = update.origin.clone();
                        current.name = update.title;
                        current.artists = update.artists;
                        current.album = update.album;
                    }
                    self.toast(message);
                    // 标签写入正文后立刻重扫本地，补全结果即时显示。
                    self.scan_local();
                } else {
                    self.toast(message);
                }
            }
            Event::TagProgress {
                seq,
                done,
                total,
                name,
            } => {
                if seq == self.tag_seq {
                    self.tag_progress = Some((done, total, name));
                }
            }
            Event::TagDone { seq, fixed, failed } => {
                if seq != self.tag_seq {
                    return;
                }
                self.tagging = false;
                self.tag_progress = None;
                self.toast(format!("批量补全完成：成功 {fixed}，失败 {failed}"));
                self.scan_local();
            }
            Event::Lyric { key, raw } => {
                let is_current = self.current.as_ref().is_some_and(|s| s.key() == key);
                if is_current && self.lyrics_key.as_deref() == Some(key.as_str()) {
                    self.lyrics = Lyrics::parse(&raw.lrc, &raw.tlyric);
                }
            }
            Event::Cover { key, result } => {
                if self.current.as_ref().is_some_and(|s| s.key() == key) {
                    match result {
                        Ok(art) => {
                            let art = Arc::new(art);
                            self.cover_cache.insert(key.clone(), art.clone());
                            self.cover = Some((key, art));
                        }
                        Err(_) => self.cover = None,
                    }
                }
            }
            Event::ApiStatus { ok, message } => {
                if !ok {
                    self.toast(message);
                }
            }
            Event::Resolved {
                seq,
                song,
                duration_ms,
                preview,
                ..
            } => {
                if seq != self.play_seq {
                    return;
                }
                let key = song.key();
                self.patch_song(
                    &key,
                    duration_ms.filter(|d| *d > 0),
                    Some(song.album.as_str()).filter(|a| !a.trim().is_empty()),
                );
                if preview {
                    self.toast("仅获取到试听片段");
                }
                self.cache_progress = Some((0, None));
            }
            Event::CacheProgress { seq, done, total } => {
                if seq != self.play_seq {
                    return;
                }
                self.cache_progress = Some((done, total));
            }
            Event::CacheReady {
                seq,
                song,
                path,
                provider: _,
            } => {
                if seq != self.play_seq {
                    return;
                }
                let start = self.next_start_ms;
                let autoplay = self.pending_autoplay;
                self.next_start_ms = 0;
                self.loading_play = false;
                self.loaded = true;
                self.playing = autoplay;
                self.cache_progress = None;
                self.engine.send(Cmd::Load {
                    path,
                    duration_ms: song.duration_ms.max(self.dur_ms),
                    start_ms: start,
                    autoplay,
                });
                if autoplay {
                    self.record_history(&song);
                }
                self.sync_mpris(true);
            }
            Event::PlayError { seq, message } => {
                if seq != self.play_seq {
                    return;
                }
                self.skip_unplayable(message);
            }
            Event::DownloadProgress { seq, done, total } => {
                if seq != self.download_seq || self.downloading.is_none() {
                    return;
                }
                self.download_progress = Some((done, total));
            }
            Event::DownloadDone { seq, path, bytes } => {
                if seq != self.download_seq {
                    return;
                }
                self.downloading = None;
                self.download_progress = None;
                self.toast(format!(
                    "下载完成（{}）：{}",
                    human_bytes(bytes),
                    path.display()
                ));
            }
            Event::DownloadFailed { seq, message } => {
                if seq != self.download_seq {
                    return;
                }
                self.downloading = None;
                self.download_progress = None;
                self.toast(format!("下载失败：{message}"));
            }
            Event::Player(event) => self.on_player(event),
            Event::Action(action) => self.handle_action(action),
        }
    }

    fn on_player(&mut self, event: PlayerEvent) {
        match event {
            PlayerEvent::Ready => {
                // 解码成功才清零；连续坏文件必须在达到上限后停下。
                self.loading_play = false;
                self.failed_streak = 0;
            }
            PlayerEvent::Ended => {
                self.pos_ms = self.dur_ms;
                self.next_song(true);
            }
            PlayerEvent::Error(message) => {
                self.skip_unplayable(message);
            }
            PlayerEvent::Position {
                pos_ms,
                dur_ms,
                playing,
            } => {
                // 加载新曲期间，引擎可能还在上报上一首的进度，忽略。
                if self.loading_play {
                    return;
                }
                self.pos_ms = pos_ms;
                if dur_ms > 0 {
                    self.dur_ms = dur_ms;
                }
                self.playing = playing;
            }
        }
    }

    fn handle_action(&mut self, action: Action) {
        match action {
            Action::TogglePlay => self.toggle_play(),
            Action::Next => self.next_song(false),
            Action::Previous => self.previous_song(),
            Action::SeekDelta(delta) => self.seek_by(delta),
            Action::SeekTo(ms) => self.seek_to(ms),
            Action::SetVolume(volume) => self.set_volume(volume),
            Action::SetMode(mode) => self.set_mode(mode),
            Action::SetShuffle(shuffle) => self.set_shuffle(shuffle),
        }
    }

    pub fn tick(&mut self) {
        while let Some(event) = self.engine.try_recv() {
            self.on_player(event);
        }
        if let Some((_, at)) = &self.toast
            && at.elapsed() > Duration::from_secs(4)
        {
            self.toast = None;
        }
        self.ensure_selection();
        self.sync_mpris(false);
    }

    // ---------- 键盘 ----------

    fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
            self.quit = true;
            return;
        }
        if self.show_help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                    self.show_help = false;
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.help_scroll = self.help_scroll.saturating_add(1);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.help_scroll = self.help_scroll.saturating_sub(1);
                }
                KeyCode::PageDown => {
                    self.help_scroll = self.help_scroll.saturating_add(5);
                }
                KeyCode::PageUp => {
                    self.help_scroll = self.help_scroll.saturating_sub(5);
                }
                KeyCode::Char('g') | KeyCode::Home => self.help_scroll = 0,
                KeyCode::Char('G') | KeyCode::End => self.help_scroll = u16::MAX,
                _ => {}
            }
            return;
        }
        if self.focus == Focus::Input {
            self.on_input_key(key);
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => {
                self.show_help = true;
                self.help_scroll = 0;
            }
            KeyCode::Char('/') => {
                self.focus = Focus::Input;
                self.input_mode = InputMode::Search;
                if let Some(search) = &self.search {
                    self.input = search.query.clone();
                } else {
                    self.input.clear();
                }
                self.input_cursor = self.input.len();
            }
            KeyCode::Char('f') => {
                self.focus = Focus::Input;
                self.input_mode = InputMode::PlaylistSearch;
                self.input = self.netease_query.clone();
                self.input_cursor = self.input.len();
            }
            KeyCode::Char('o') => {
                self.focus = Focus::Input;
                self.input_mode = InputMode::Open;
                self.input.clear();
                self.input_cursor = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('g') | KeyCode::Home => self.select_boundary(false),
            KeyCode::Char('G') | KeyCode::End => self.select_boundary(true),
            KeyCode::PageUp => self.scroll_lyrics(-5),
            KeyCode::PageDown => self.scroll_lyrics(5),
            KeyCode::Enter => self.activate_selected(),
            KeyCode::Char(' ') => self.toggle_play(),
            KeyCode::Char(']') => self.next_song(false),
            KeyCode::Char('[') => self.previous_song(),
            KeyCode::Left => self.seek_by(-5000),
            KeyCode::Right => self.seek_by(5000),
            KeyCode::Char('+') | KeyCode::Char('=') => {
                let volume = self.volume + 0.05;
                self.set_volume(volume);
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                let volume = self.volume - 0.05;
                self.set_volume(volume);
            }
            KeyCode::Char('r') => self.cycle_mode(),
            KeyCode::Char('d') => self.download_current(),
            KeyCode::Char('l') => {
                self.show_lyrics = !self.show_lyrics;
                // 关闭歌词时同步退出纯享模式，避免歌词区仍被纯享占据。
                if !self.show_lyrics {
                    self.lyrics_immersive = false;
                }
                self.toast(if self.show_lyrics {
                    "歌词：开"
                } else {
                    "歌词：关"
                });
            }
            KeyCode::Char('v') => {
                self.lyrics_immersive = !self.lyrics_immersive;
                // 进入纯享时强制打开歌词；退出纯享回到分栏形态。
                if self.lyrics_immersive {
                    self.show_lyrics = true;
                }
                self.toast(if self.lyrics_immersive {
                    "歌词纯享：开"
                } else {
                    "歌词纯享：关"
                });
            }
            KeyCode::Char('R') => self.refresh_current(),
            KeyCode::Char('t') => self.tag_selected(),
            KeyCode::Char('T') => self.tag_all_missing(),
            KeyCode::Char('x') | KeyCode::Delete => self.remove_history_selected(),
            KeyCode::Esc => self.go_back(),
            _ => {}
        }
    }

    /// Esc 逐级返回：搜索 → 网易 / 雀乐层级 → 首页。
    fn go_back(&mut self) {
        if self.search.is_some() {
            self.remember_selection();
            self.search_seq += 1;
            self.search = None;
            self.search_loading = false;
            self.error = None;
            self.restore_selection();
            return;
        }
        match self.view {
            View::Netease
                if self.netease_page == NeteasePage::Tracks && self.netease_back_to_search =>
            {
                self.remember_selection();
                self.netease_seq += 1;
                self.netease_loading = false;
                self.netease_page = NeteasePage::Search;
                self.netease_back_to_search = false;
                self.error = None;
                self.restore_selection();
            }
            View::Netease if self.netease_page != NeteasePage::Browse => {
                self.remember_selection();
                self.netease_seq += 1;
                self.netease_loading = false;
                self.netease_page = NeteasePage::Browse;
                self.netease_back_to_search = false;
                self.error = None;
                self.restore_selection();
            }
            View::Indie if self.indie_page != IndiePage::Playlists => {
                self.remember_selection();
                self.indie_page = IndiePage::Playlists;
                self.error = None;
                self.restore_selection();
            }
            View::Home => {}
            _ => self.go_home(),
        }
    }

    fn on_input_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => {
                self.focus = Focus::List;
                if self.input_mode == InputMode::Open {
                    self.input.clear();
                    self.input_cursor = 0;
                    self.input_mode = InputMode::Search;
                }
            }
            KeyCode::Enter => self.submit_input(),
            KeyCode::Backspace => self.delete_previous(),
            KeyCode::Delete => self.delete_next(),
            KeyCode::Left => self.input_cursor = prev_boundary(&self.input, self.input_cursor),
            KeyCode::Right => self.input_cursor = next_boundary(&self.input, self.input_cursor),
            KeyCode::Home => self.input_cursor = 0,
            KeyCode::End => self.input_cursor = self.input.len(),
            KeyCode::Char('u') if ctrl => {
                self.input.clear();
                self.input_cursor = 0;
            }
            KeyCode::Char(ch) if !ctrl && !alt => {
                self.input.insert(self.input_cursor, ch);
                self.input_cursor += ch.len_utf8();
                if self.input.capacity() > 4096 {
                    self.input.shrink_to_fit();
                }
            }
            _ => {}
        }
    }

    fn delete_previous(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        let start = prev_boundary(&self.input, self.input_cursor);
        self.input.replace_range(start..self.input_cursor, "");
        self.input_cursor = start;
    }

    fn delete_next(&mut self) {
        if self.input_cursor >= self.input.len() {
            return;
        }
        let end = next_boundary(&self.input, self.input_cursor);
        self.input.replace_range(self.input_cursor..end, "");
    }

    // ---------- 渲染 ----------

    pub fn render(&mut self, frame: &mut Frame) {
        self.ensure_selection();
        let area = frame.area();
        if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
            ui::render_too_small(frame, area, &THEME);
            return;
        }

        let search_visible = self.focus == Focus::Input;
        let search_h = u16::from(search_visible);
        let player_h = 2;
        let fixed = search_h + player_h;
        let available = area.height.saturating_sub(fixed);
        // 歌词纯享模式：歌词面板向上拓展到全部内容区；搜索时让位给搜索结果。
        let immersive = self.lyrics_immersive && self.search.is_none();
        let home = !immersive && self.view == View::Home && self.search.is_none();
        let (list_h, lyrics_h) = if immersive {
            (0, available)
        } else if home {
            // 首页歌词关闭时，主页内容在整个主体区垂直居中；
            // 歌词打开时保持紧凑 9 行（含顶部一行空行）在上方，歌词拿剩余行数。
            if self.show_lyrics {
                let list = 9.min(available);
                (list, available - list)
            } else {
                (available, 0)
            }
        } else if self.show_lyrics {
            let min_list = 5.min(available);
            let max_list = available.saturating_sub(5).max(min_list);
            let desired = ((available as u32 * 40 / 100) as u16).max(min_list);
            let list = desired.min(max_list).min(available);
            (list, available - list)
        } else {
            (available, 0)
        };
        // 歌词区用 Min 承接所有剩余行数：歌词关闭时也能保证播放条紧贴底部。
        let chunks = Layout::vertical([
            Constraint::Length(search_h),
            Constraint::Length(list_h),
            Constraint::Min(lyrics_h),
            Constraint::Length(player_h),
        ])
        .split(area);
        let search_area = chunks[0];
        let list_area = chunks[1];
        let lyrics_area = chunks[2];
        let player_area = chunks[3];

        if search_visible {
            ui::render_search(
                frame,
                search_area,
                &THEME,
                &self.input,
                self.input_cursor,
                self.input_mode,
                self.focus,
                false,
            );
        }

        let title = self.current_list_title();
        let current_key = self.current.as_ref().map(|song| song.key());
        let loading = self.loading();
        let empty = self.empty_message();
        let error = self.error.as_deref();

        if immersive {
            // 纯享模式：列表区不渲染，整个内容区交给歌词。
        } else if let Some(search) = &self.search {
            ui::render_songs(
                frame,
                list_area,
                &THEME,
                &search.songs,
                current_key.as_deref(),
                &mut self.list_state,
                &title,
                loading,
                error,
                empty,
            );
        } else {
            match self.view {
                View::Home => {
                    let labels: Vec<&str> = HOME_ENTRIES.iter().map(|v| v.label()).collect();
                    ui::render_home(
                        frame,
                        list_area,
                        &THEME,
                        &labels,
                        &mut self.list_state,
                        &title,
                    );
                }
                View::Netease => match self.netease_page {
                    NeteasePage::Browse => ui::render_songs(
                        frame,
                        list_area,
                        &THEME,
                        &self.netease_chart,
                        current_key.as_deref(),
                        &mut self.list_state,
                        &title,
                        loading,
                        error,
                        empty,
                    ),
                    NeteasePage::Search => ui::render_playlists(
                        frame,
                        list_area,
                        &THEME,
                        &self.netease_playlist_hits,
                        &mut self.list_state,
                        &title,
                        loading,
                        error,
                        empty,
                    ),
                    NeteasePage::Tracks => {
                        let songs = self
                            .netease_tracks
                            .as_ref()
                            .map(|t| t.songs.as_slice())
                            .unwrap_or(&[]);
                        ui::render_songs(
                            frame,
                            list_area,
                            &THEME,
                            songs,
                            current_key.as_deref(),
                            &mut self.list_state,
                            &title,
                            loading,
                            error,
                            empty,
                        );
                    }
                },
                View::Indie => match self.indie_page {
                    IndiePage::Playlists => {
                        let playlists: Vec<Playlist> = self
                            .indie_playlists
                            .iter()
                            .map(|p| Playlist {
                                id: p.id.clone(),
                                name: p.name.clone(),
                                cover_url: p.cover_url.clone(),
                                track_count: p.songs.len() as u64,
                                play_count: 0,
                                creator: "雀乐".into(),
                                description: String::new(),
                            })
                            .collect();
                        ui::render_playlists(
                            frame,
                            list_area,
                            &THEME,
                            &playlists,
                            &mut self.list_state,
                            &title,
                            loading,
                            error,
                            empty,
                        );
                    }
                    IndiePage::Tracks => {
                        let songs = self
                            .indie_tracks
                            .as_ref()
                            .map(|t| t.songs.as_slice())
                            .unwrap_or(&[]);
                        ui::render_songs(
                            frame,
                            list_area,
                            &THEME,
                            songs,
                            current_key.as_deref(),
                            &mut self.list_state,
                            &title,
                            loading,
                            error,
                            empty,
                        );
                    }
                },
                View::StreetVoiceRecommend => ui::render_songs(
                    frame,
                    list_area,
                    &THEME,
                    &self.sv_recommend,
                    current_key.as_deref(),
                    &mut self.list_state,
                    &title,
                    loading,
                    error,
                    empty,
                ),
                View::Local => ui::render_songs(
                    frame,
                    list_area,
                    &THEME,
                    &self.local_songs,
                    current_key.as_deref(),
                    &mut self.list_state,
                    &title,
                    loading,
                    error,
                    empty,
                ),
                View::Recent => ui::render_songs(
                    frame,
                    list_area,
                    &THEME,
                    &self.history,
                    current_key.as_deref(),
                    &mut self.list_state,
                    &title,
                    loading,
                    error,
                    empty,
                ),
            }
        }

        if immersive || self.show_lyrics {
            ui::render_lyrics(
                frame,
                lyrics_area,
                &THEME,
                &self.lyrics,
                self.pos_ms,
                self.cover.as_ref().map(|(_, art)| art.as_ref()),
                &mut self.lyrics_scroll,
            );
        }

        let toast = self.toast.as_ref().and_then(|(message, at)| {
            (at.elapsed() < Duration::from_secs(4)).then_some(message.as_str())
        });
        let hint = ui::status_hint(
            toast,
            error,
            self.tag_progress
                .as_ref()
                .map(|(done, total, name)| (*done, *total, name.as_str())),
            self.cache_progress,
            self.download_progress,
        );
        ui::render_player(
            frame,
            player_area,
            &THEME,
            self.current.as_ref(),
            self.playing,
            self.loading_play,
            self.pos_ms,
            self.dur_ms,
            self.mode,
            hint,
        );

        if self.show_help {
            ui::render_help(frame, area, &THEME, &mut self.help_scroll);
        }
    }

    // ---------- MPRIS / 配置 ----------

    fn start_mpris(&mut self) {
        if self.mpris_cmd.is_some() {
            return;
        }
        self.mpris_cmd = mpris::spawn(self.tx.clone());
        if self.mpris_cmd.is_none() {
            self.toast("MPRIS 启动失败（请检查 D-Bus session）");
        }
        self.mpris_last_sync = Instant::now()
            .checked_sub(Duration::from_secs(5))
            .unwrap_or_else(Instant::now);
        self.sync_mpris(true);
    }

    fn sync_mpris(&mut self, force: bool) {
        let Some(tx) = &self.mpris_cmd else {
            return;
        };
        if !force && self.mpris_last_sync.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.mpris_last_sync = Instant::now();
        let (title, artists, album, art_url) = match &self.current {
            Some(song) => (
                song.name.clone(),
                song.artists.clone(),
                song.album.clone(),
                song.cover_url.clone(),
            ),
            None => (String::new(), Vec::new(), String::new(), None),
        };
        let loop_status = match self.mode {
            PlayMode::Single => MprisLoop::Track,
            PlayMode::Sequential => MprisLoop::None,
            PlayMode::ListLoop | PlayMode::Shuffle => MprisLoop::Playlist,
        };
        let state = MprisState {
            title,
            artists,
            album,
            duration_ms: self.dur_ms,
            position_ms: self.pos_ms,
            art_url,
            playing: self.playing,
            stopped: self.current.is_none(),
            volume: self.volume,
            loop_status,
            shuffle: self.mode == PlayMode::Shuffle,
            can_next: !self.queue.is_empty(),
            can_prev: !self.queue.is_empty(),
        };
        let _ = tx.send(MprisCmd::Update(Box::new(state)));
    }

    fn save_session(&self) {
        if !self.cfg.ui.restore_session {
            return;
        }
        let session = SessionFile {
            queue: self.queue.clone(),
            index: self.cursor,
            pos_ms: self.pos_ms,
        };
        match serde_json::to_string_pretty(&session) {
            Ok(json) => {
                let _ = Paths::ensure_parent(&self.paths.session_file);
                if let Err(e) = std::fs::write(&self.paths.session_file, json) {
                    tracing::warn!("保存会话失败 {}: {e}", self.paths.session_file.display());
                }
            }
            Err(e) => tracing::warn!("序列化会话失败: {e}"),
        }
    }

    pub fn finish(&mut self) {
        self.cfg.player.volume = self.volume;
        self.cfg.player.quality = self.quality;
        self.cfg.player.mode = self.mode;
        if let Err(e) = self.cfg.save(&self.paths.config_file) {
            tracing::warn!("保存配置失败: {e}");
        }
        self.save_session();
    }

    fn toast(&mut self, message: impl Into<String>) {
        self.toast = Some((message.into(), Instant::now()));
    }
}

fn load_session(path: &Path) -> Option<SessionFile> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn local_origin(song: &Song) -> PathBuf {
    song.origin
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&song.id))
}

fn same_dir(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// 复制原始文件到转换目录，写入标签 / 歌词 / 封面，再原子改名。
/// 原始文件始终保持不动。
async fn convert_and_tag(
    api: &ApiClient,
    http: &reqwest::Client,
    local: &Song,
    matched: &Song,
    source_dir: &Path,
    output_dir: &Path,
) -> std::result::Result<TagUpdate, String> {
    if same_dir(source_dir, output_dir) {
        return Err("转换目录不能与源目录相同，请修改配置 [local] output_dir".into());
    }
    let origin = local_origin(local);
    if !origin.exists() {
        return Err(format!("原始文件不存在：{}", origin.display()));
    }
    if origin.starts_with(output_dir) {
        return Err("该文件已经在转换目录里".into());
    }
    let relative = origin.strip_prefix(source_dir).map_or_else(
        |_| {
            origin
                .file_name()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("unknown"))
        },
        Path::to_path_buf,
    );
    let dest = output_dir.join(relative);

    let lyric = match api.lyric(&matched.id).await {
        Ok(raw) if !raw.lrc.trim().is_empty() => Some(raw.lrc),
        _ => None,
    };
    let cover = match matched
        .cover_url
        .as_deref()
        .filter(|u| !u.trim().is_empty())
    {
        Some(url) => match http
            .get(url)
            .header("Referer", "https://music.163.com/")
            .send()
            .await
        {
            Ok(resp) => resp
                .bytes()
                .await
                .ok()
                .map(|bytes| bytes.to_vec())
                .filter(|bytes| !bytes.is_empty()),
            Err(_) => None,
        },
        None => None,
    };

    let title = matched.name.clone();
    let artists = matched.artists.clone();
    let album = matched.album.clone();
    let dest_label = dest.to_string_lossy().into_owned();
    let origin_label = origin.to_string_lossy().into_owned();
    let dest_io = dest.clone();
    let origin_io = origin.clone();
    let title_io = title.clone();
    let artists_io = artists.clone();
    let album_io = album.clone();

    tokio::task::spawn_blocking(move || -> std::result::Result<(), String> {
        if let Some(parent) = dest_io.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
        }
        let ext = dest_io
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_else(|| "part".to_string());
        let temp = dest_io.with_extension(format!("part.{ext}"));
        let _ = std::fs::remove_file(&temp);
        std::fs::copy(&origin_io, &temp).map_err(|e| format!("复制文件失败：{e}"))?;
        if let Err(e) = musicyi_library::write_meta_full(
            &temp,
            Some(&title_io),
            &artists_io,
            Some(&album_io),
            lyric.as_deref(),
            cover.as_deref(),
        ) {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("写入标签失败：{e}"));
        }
        if let Err(e) = std::fs::rename(&temp, &dest_io) {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("重命名失败：{e}"));
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("转换任务失败：{e}"))??;

    Ok(TagUpdate {
        title,
        artists,
        album,
        id: dest_label,
        origin: Some(origin_label),
    })
}

fn decode_cover(bytes: &[u8]) -> Option<CoverArt> {
    let image = image::load_from_memory(bytes).ok()?;
    let thumb = image.thumbnail(72, 72);
    let rgb = thumb.to_rgb8();
    let (width, height) = (rgb.width(), rgb.height());
    if width == 0 || height == 0 {
        return None;
    }
    Some(CoverArt {
        width,
        height,
        rgb: rgb.into_raw(),
    })
}

fn local_song(path: &Path, title: Option<String>, duration_ms: u64) -> Song {
    local_song_at(path, path, title, duration_ms)
}

/// `play_path` 是实际播放的文件（优先转换副本），`origin_path` 是扫描到的
/// 原始文件，只用来推断标题 / 歌手，绝不写入。
fn local_song_at(
    play_path: &Path,
    origin_path: &Path,
    title: Option<String>,
    duration_ms: u64,
) -> Song {
    let meta = musicyi_library::read_meta(play_path);
    let (file_artist, file_title) = musicyi_library::filename_hint(origin_path);
    let converted = play_path != origin_path;
    let album = if converted {
        meta.as_ref()
            .and_then(|m| m.album.clone())
            .unwrap_or_default()
    } else {
        meta.as_ref()
            .map(|m| display_album(m, file_artist.as_deref(), file_title.as_deref()))
            .unwrap_or_default()
    };
    let meta_artists: Vec<String> = meta
        .as_ref()
        .map(|m| {
            m.artists
                .iter()
                .filter(|a| a.as_str() != "未知歌手" && !a.trim().is_empty())
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    // 原始文件名 `歌手 - 标题` 比内嵌标签可靠：多数文件的标签字段存在整组错位。
    // 转换副本是刚写入的正式标签，优先使用标签。
    let artists = if converted {
        meta_artists.clone()
    } else {
        file_artist
            .map(|artist| vec![artist])
            .filter(|artists| !artists.is_empty())
            .unwrap_or(meta_artists)
    };
    let name = if converted {
        meta.as_ref()
            .and_then(|m| m.title.clone())
            .filter(|s| !s.trim().is_empty())
            .or(title)
            .or(file_title)
    } else {
        title.or(file_title).or_else(|| {
            meta.as_ref()
                .and_then(|m| m.title.clone())
                .filter(|s| !s.trim().is_empty())
        })
    }
    .or_else(|| {
        origin_path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
    })
    .unwrap_or_else(|| "未知曲目".into());
    let meta_duration = meta.as_ref().map(|m| m.duration_ms).unwrap_or(0);
    Song {
        id: play_path.to_string_lossy().into_owned(),
        name,
        artists,
        album,
        album_id: None,
        duration_ms: if duration_ms > 0 {
            duration_ms
        } else {
            meta_duration
        },
        cover_url: None,
        fee: 0,
        source: SourceKind::Local,
        play_url: None,
        lyric_url: None,
        origin: Some(origin_path.to_string_lossy().into_owned()),
    }
}

/// 本地文件经常把歌手、专辑、标题三个字段整组错位。若标签呈现典型轮换
/// （title=歌手 或 album=歌手），把标签里的 artist 字段当作真正的专辑。
fn display_album(
    meta: &musicyi_library::EmbeddedMeta,
    hint_artist: Option<&str>,
    hint_title: Option<&str>,
) -> String {
    let album = meta.album.as_deref().map(str::trim).unwrap_or("");
    let title = meta.title.as_deref().map(str::trim).unwrap_or("");
    let (Some(hint_artist), Some(hint_title)) = (
        hint_artist.map(str::trim).filter(|s| !s.is_empty()),
        hint_title.map(str::trim).filter(|s| !s.is_empty()),
    ) else {
        return album.to_string();
    };
    let rotated = (same_text(title, hint_artist) && same_text(album, hint_title))
        || (same_text(title, hint_title) && same_text(album, hint_artist));
    if rotated {
        let artist_album = meta
            .artists
            .iter()
            .filter(|a| a.as_str() != "未知歌手" && !a.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" / ");
        if !artist_album.is_empty() {
            return artist_album;
        }
    }
    album.to_string()
}

fn same_text(left: &str, right: &str) -> bool {
    !left.is_empty() && !right.is_empty() && normalize(left) == normalize(right)
}

fn scan_local_dir(dir: &Path, output_dir: &Path) -> std::result::Result<LocalScan, String> {
    if !dir.exists() {
        return Err(format!("目录不存在：{}", dir.display()));
    }
    if !dir.is_dir() {
        return Err(format!("不是目录：{}", dir.display()));
    }
    let mut files = Vec::new();
    collect_audio_files(dir, output_dir, &mut files);
    files.sort();
    let mut scan = LocalScan::default();
    for path in &files {
        let relative = path.strip_prefix(dir).unwrap_or(path);
        let converted = output_dir.join(relative);
        let play = if converted.exists() {
            converted.as_path()
        } else {
            path.as_path()
        };
        let song = local_song_at(play, path, None, 0);
        let needs_tag = play == path
            || musicyi_library::read_meta(play)
                .is_none_or(|m| m.title.as_deref().is_none_or(|t| t.trim().is_empty()));
        if needs_tag {
            // 还没有转换副本的文件，也列入批量补全范围。
            scan.needs_tag.push(song.id.clone());
        }
        scan.songs.push(song);
    }
    Ok(scan)
}

fn collect_audio_files(dir: &Path, output_dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            // 转换输出目录不参与扫描，否则会把副本当成新歌。
            if path.starts_with(output_dir) {
                continue;
            }
            collect_audio_files(&path, output_dir, out);
        } else if file_type.is_file() && input::is_audio_path(&path) && !input::is_m3u_path(&path) {
            out.push(path);
        }
    }
}

fn normalize(text: &str) -> String {
    text.chars()
        .flat_map(|c| c.to_lowercase())
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn pick_best_match(
    song: &Song,
    hint_artist: Option<&str>,
    hint_title: Option<&str>,
    candidates: &[Song],
) -> Option<Song> {
    let want_title = hint_title
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| song.name.clone());
    let want_artist = hint_artist
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| song.artists.first().cloned())
        .unwrap_or_default();
    let want_title_n = normalize(&want_title);
    let want_artist_n = normalize(&want_artist);
    if want_title_n.is_empty() {
        return None;
    }

    let mut best: Option<(i32, Song)> = None;
    for candidate in candidates {
        let title_n = normalize(&candidate.name);
        let title_score = if title_n == want_title_n {
            100
        } else if !title_n.is_empty()
            && (title_n.contains(&want_title_n) || want_title_n.contains(&title_n))
        {
            60
        } else {
            0
        };
        if title_score == 0 {
            continue;
        }
        let artist_n = normalize(&candidate.artist_text());
        let artist_score = if want_artist_n.is_empty() || artist_n.is_empty() {
            0
        } else if artist_n == want_artist_n {
            40
        } else if artist_n.contains(&want_artist_n) || want_artist_n.contains(&artist_n) {
            25
        } else {
            0
        };
        // 标题不完全一致又对不上歌手，宁可不动文件。
        if !want_artist_n.is_empty() && artist_score == 0 && title_score < 100 {
            continue;
        }
        let mut score = title_score + artist_score;
        if song.duration_ms > 0 && candidate.duration_ms > 0 {
            let diff = song.duration_ms.abs_diff(candidate.duration_ms);
            score += if diff <= 2000 {
                20
            } else if diff <= 5000 {
                10
            } else if diff <= 10000 {
                0
            } else {
                -25
            };
        }
        // 本地文件名能给出歌手时，必须同时有歌手或时长证据，避免只凭
        // 同名标题把另一首歌的标签写进来。
        let threshold = if want_artist_n.is_empty() { 90 } else { 120 };
        if score < threshold {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|(best_score, _)| score > *best_score)
        {
            best = Some((score, candidate.clone()));
        }
    }
    best.map(|(_, song)| song)
}

fn prev_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor.min(text.len())]
        .char_indices()
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn next_boundary(text: &str, cursor: usize) -> usize {
    let cursor = cursor.min(text.len());
    text[cursor..]
        .chars()
        .next()
        .map(|c| cursor + c.len_utf8())
        .unwrap_or(text.len())
}

fn human_bytes(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= MB {
        format!("{:.1} MB", bytes as f64 / MB)
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(name: &str, artist: &str, duration_ms: u64) -> Song {
        Song {
            id: name.into(),
            name: name.into(),
            artists: vec![artist.into()],
            duration_ms,
            ..Song::default()
        }
    }

    #[test]
    fn best_match_prefers_title_and_artist() {
        let local = song("Ground Zero", "April Rain", 372_000);
        let candidates = vec![
            song("Ground Zero", "Some Cover Band", 200_000),
            song("Ground Zero", "April Rain", 372_400),
        ];
        let matched = pick_best_match(&local, Some("April Rain"), Some("Ground Zero"), &candidates)
            .expect("应匹配到原曲");
        assert_eq!(matched.artists, vec!["April Rain"]);
    }

    #[test]
    fn best_match_rejects_unrelated() {
        let local = song("Ground Zero", "April Rain", 372_000);
        let candidates = vec![song("完全无关", "别的歌手", 100_000)];
        assert!(pick_best_match(&local, None, None, &candidates).is_none());
    }

    #[test]
    fn best_match_needs_artist_or_duration_evidence() {
        let local = song("Ground Zero", "April Rain", 372_000);
        let wrong_artist = vec![song("Ground Zero", "Some Cover Band", 0)];
        assert!(pick_best_match(&local, None, None, &wrong_artist).is_none());
        let same_duration = vec![song("Ground Zero", "Some Cover Band", 372_200)];
        assert!(pick_best_match(&local, None, None, &same_duration).is_some());
    }
}
