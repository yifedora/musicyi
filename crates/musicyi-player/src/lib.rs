//! 音频引擎：rodio 播放 + 下载缓存。
//!
//! rodio 的 `Player` 不是 async 友好的，所以音频线程独立运行，
//! 通过 std channel 与 UI/事件循环通信。

pub mod cache;
pub mod engine;

pub use cache::AudioCache;
pub use engine::{AudioEngine, Cmd, PlayerEvent};
