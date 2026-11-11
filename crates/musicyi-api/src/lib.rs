//! 网易云接口客户端 + 音源解析。
//!
//! 只实现 `external` 后端：本机 netease-cloud-music-api。

pub mod client;
pub mod dto;
pub mod resolver;

pub use client::{ApiClient, RawLyric, SongUrl, StreetVoiceDetail};
pub use dto::IndiePlaylist;
pub use resolver::{ResolvedSource, Resolver};
