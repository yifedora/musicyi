use serde::{Deserialize, Serialize};

/// 音源类型。`Netease` 走接口返回的直链，其余为解析降级链。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    #[default]
    Netease,
    Gdstudio,
    Custom,
    Local,
    /// 雀乐（indie.cn）歌单里的直接 MP3。
    Indie,
    /// 街声（streetvoice.cn）推荐，播放时经本机 API 转成 MP3。
    StreetVoice,
}

impl SourceKind {
    pub fn label(&self) -> &'static str {
        match self {
            SourceKind::Netease => "网易云",
            SourceKind::Gdstudio => "GD音乐台",
            SourceKind::Custom => "自定义",
            SourceKind::Local => "本地",
            SourceKind::Indie => "雀乐",
            SourceKind::StreetVoice => "街声",
        }
    }

    /// 在线来源才需要走解析器；本地文件直接交给音频引擎。
    pub fn is_online(&self) -> bool {
        !matches!(self, SourceKind::Local)
    }
}

/// 一首歌（在线或本地统一表示）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Song {
    pub id: String,
    pub name: String,
    pub artists: Vec<String>,
    pub album: String,
    pub album_id: Option<String>,
    pub duration_ms: u64,
    pub cover_url: Option<String>,
    pub fee: i64,
    pub source: SourceKind,
    /// 雀乐等来源提供的可直接播放地址。
    #[serde(default)]
    pub play_url: Option<String>,
    /// 雀乐等来源提供的 LRC 地址。
    #[serde(default)]
    pub lyric_url: Option<String>,
    /// 本地文件的实际来源路径。`id` 指向转换 / 补全后的播放文件，
    /// `origin` 始终记录扫描目录里的原始文件，便于重新生成。
    #[serde(default)]
    pub origin: Option<String>,
}

impl Song {
    pub fn key(&self) -> String {
        format!("{}:{}", self.source.label(), self.id)
    }

    pub fn artist_text(&self) -> String {
        if self.artists.is_empty() {
            "未知歌手".to_string()
        } else {
            self.artists.join(" / ")
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub cover_url: Option<String>,
    pub track_count: u64,
    pub play_count: u64,
    pub creator: String,
    pub description: String,
}

/// 请求音质。与网易云 `/song/url/v1` 的 level 参数一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Standard,
    Higher,
    #[default]
    ExHigh,
    Lossless,
    Hires,
}

impl Quality {
    pub fn level(&self) -> &'static str {
        match self {
            Quality::Standard => "standard",
            Quality::Higher => "higher",
            Quality::ExHigh => "exhigh",
            Quality::Lossless => "lossless",
            Quality::Hires => "hires",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Quality::Standard => "标准 128k",
            Quality::Higher => "较高 192k",
            Quality::ExHigh => "极高 320k",
            Quality::Lossless => "无损 FLAC",
            Quality::Hires => "Hi-Res",
        }
    }

    /// 降一档，用于官方直链拿不到时自动回退。
    pub fn downgrade(&self) -> Option<Quality> {
        match self {
            Quality::Hires => Some(Quality::Lossless),
            Quality::Lossless => Some(Quality::ExHigh),
            Quality::ExHigh => Some(Quality::Higher),
            Quality::Higher => Some(Quality::Standard),
            Quality::Standard => None,
        }
    }

    pub fn from_level(level: &str) -> Quality {
        match level.to_ascii_lowercase().as_str() {
            "standard" => Quality::Standard,
            "higher" => Quality::Higher,
            "lossless" => Quality::Lossless,
            "hires" => Quality::Hires,
            _ => Quality::ExHigh,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlayMode {
    #[default]
    ListLoop,
    Single,
    Shuffle,
    Sequential,
}

impl PlayMode {
    pub fn label(&self) -> &'static str {
        match self {
            PlayMode::ListLoop => "列表循环",
            PlayMode::Single => "单曲循环",
            PlayMode::Shuffle => "随机播放",
            PlayMode::Sequential => "顺序播放",
        }
    }

    /// 播放条右侧使用的短标签。
    pub fn short_label(&self) -> &'static str {
        match self {
            PlayMode::ListLoop => "循环",
            PlayMode::Single => "单曲",
            PlayMode::Shuffle => "随机",
            PlayMode::Sequential => "顺序",
        }
    }

    pub fn cycle(&self) -> PlayMode {
        match self {
            PlayMode::ListLoop => PlayMode::Single,
            PlayMode::Single => PlayMode::Shuffle,
            PlayMode::Shuffle => PlayMode::Sequential,
            PlayMode::Sequential => PlayMode::ListLoop,
        }
    }
}
