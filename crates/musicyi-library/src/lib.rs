//! 本地文件与下载落盘：读取标签、把当前歌曲保存到音乐目录。

pub mod download;
pub mod tags;

pub use download::{DownloadOutcome, download_to, sanitize};
pub use tags::{
    EmbeddedMeta, clean_match_title, filename_hint, read_cover, read_lyrics, read_meta,
    tag_needs_fix, write_meta, write_meta_full,
};
