use musicyi_core::model::PlayMode;

/// 单一首页列表中的五个入口；选中后进入对应内容列表。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Home,
    Netease,
    StreetVoiceRecommend,
    Indie,
    Local,
    Recent,
}

impl View {
    pub fn label(self) -> &'static str {
        match self {
            View::Home => "首页",
            View::Netease => "网易原创",
            View::StreetVoiceRecommend => "街声推荐",
            View::Indie => "雀乐推荐",
            View::Local => "本地列表",
            View::Recent => "最近播放",
        }
    }
}

/// 首页入口顺序。
pub const HOME_ENTRIES: [View; 5] = [
    View::Netease,
    View::StreetVoiceRecommend,
    View::Indie,
    View::Local,
    View::Recent,
];

/// 键盘焦点：输入框或列表。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    List,
    Input,
}

/// 输入框当前用途，只影响标题与提示。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    #[default]
    Search,
    PlaylistSearch,
    Open,
}

impl InputMode {
    pub fn label(self) -> &'static str {
        match self {
            InputMode::Search => "搜索单曲",
            InputMode::PlaylistSearch => "搜索歌单",
            InputMode::Open => "打开",
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            InputMode::Search => "歌曲关键词…",
            InputMode::PlaylistSearch => "网易云歌单关键词…",
            InputMode::Open => "路径 / 链接 / ID / M3U…",
        }
    }
}

/// 解码后的封面缩略图，供像素风渲染使用。
#[derive(Debug, Clone)]
pub struct CoverArt {
    pub width: u32,
    pub height: u32,
    /// RGB8，长度始终为 `width * height * 3`。
    pub rgb: Vec<u8>,
}

/// MPRIS 回调投递到主循环的动作。
#[derive(Debug, Clone)]
pub enum Action {
    TogglePlay,
    Next,
    Previous,
    SeekDelta(i64),
    SeekTo(u64),
    SetVolume(f32),
    SetMode(PlayMode),
    SetShuffle(bool),
}
