//! MPRIS 服务：把播放状态暴露给桌面媒体控件（playerctl、GNOME/KDE 媒体键等）。
//!
//! `mpris-server::Player` 内部使用 `Rc`，不是 `Send`，所以单独开一个线程，
//! 在 current_thread runtime + LocalSet 里运行；与 TUI 之间通过 channel 通信。

#[cfg(target_os = "linux")]
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;

use crate::event::Event;
#[cfg(target_os = "linux")]
use crate::types::Action;

#[cfg(target_os = "linux")]
use std::collections::hash_map::DefaultHasher;
#[cfg(target_os = "linux")]
use std::hash::{Hash, Hasher};

#[cfg(target_os = "linux")]
use mpris_server::{LoopStatus, Metadata, PlaybackStatus, Player, Time, TrackId};

/// 一次性推给 MPRIS 的完整播放状态。
#[derive(Debug, Clone, Default)]
pub struct MprisState {
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub duration_ms: u64,
    pub position_ms: u64,
    pub art_url: Option<String>,
    pub playing: bool,
    pub stopped: bool,
    pub volume: f32,
    pub loop_status: MprisLoop,
    pub shuffle: bool,
    pub can_next: bool,
    pub can_prev: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MprisLoop {
    #[default]
    None,
    Track,
    Playlist,
}

#[cfg(target_os = "linux")]
impl From<MprisLoop> for LoopStatus {
    fn from(value: MprisLoop) -> Self {
        match value {
            MprisLoop::None => LoopStatus::None,
            MprisLoop::Track => LoopStatus::Track,
            MprisLoop::Playlist => LoopStatus::Playlist,
        }
    }
}

#[derive(Debug, Clone)]
pub enum MprisCmd {
    Update(Box<MprisState>),
    Seeked(u64),
}

/// 启动 MPRIS 服务。失败（例如没有 D-Bus）时返回 None，不影响主程序。
#[cfg(target_os = "linux")]
pub fn spawn(tx: UnboundedSender<Event>) -> Option<UnboundedSender<MprisCmd>> {
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
    let builder = std::thread::Builder::new().name("musicyi-mpris".into());
    let handle = builder.spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                tracing::warn!("创建 MPRIS runtime 失败: {e}");
                return;
            }
        };
        let local = tokio::task::LocalSet::new();
        local.block_on(&runtime, run(tx, cmd_rx));
    });
    match handle {
        Ok(_) => Some(cmd_tx),
        Err(e) => {
            tracing::warn!("启动 MPRIS 线程失败: {e}");
            None
        }
    }
}

/// 非 Linux 平台没有 D-Bus / MPRIS，返回 None 让上层保持无媒体集成。
#[cfg(not(target_os = "linux"))]
pub fn spawn(_tx: UnboundedSender<Event>) -> Option<UnboundedSender<MprisCmd>> {
    None
}

#[cfg(target_os = "linux")]
async fn run(tx: UnboundedSender<Event>, mut cmd_rx: UnboundedReceiver<MprisCmd>) {
    let player = match Player::builder("musicyi")
        .identity("musicyi")
        .desktop_entry("musicyi")
        .can_quit(false)
        .can_raise(false)
        .can_play(true)
        .can_pause(true)
        .can_go_next(true)
        .can_go_previous(true)
        .can_seek(true)
        .can_control(true)
        .build()
        .await
    {
        Ok(player) => player,
        Err(e) => {
            tracing::warn!("MPRIS 服务启动失败（可能是没有 D-Bus session）: {e}");
            return;
        }
    };

    let tx_play = tx.clone();
    player.connect_play_pause(move |_| {
        let _ = tx_play.send(Event::Action(Action::TogglePlay));
    });
    let tx_next = tx.clone();
    player.connect_next(move |_| {
        let _ = tx_next.send(Event::Action(Action::Next));
    });
    let tx_prev = tx.clone();
    player.connect_previous(move |_| {
        let _ = tx_prev.send(Event::Action(Action::Previous));
    });
    let tx_seek = tx.clone();
    player.connect_seek(move |_, offset| {
        let ms = offset.as_millis();
        if ms != 0 {
            let _ = tx_seek.send(Event::Action(Action::SeekDelta(ms)));
        }
    });
    let tx_pos = tx.clone();
    player.connect_set_position(move |_, _track, pos| {
        let ms = pos.as_millis().max(0) as u64;
        let _ = tx_pos.send(Event::Action(Action::SeekTo(ms)));
    });
    let tx_vol = tx.clone();
    player.connect_set_volume(move |_, volume| {
        let _ = tx_vol.send(Event::Action(Action::SetVolume(volume as f32)));
    });
    let tx_loop = tx.clone();
    player.connect_set_loop_status(move |_, status| {
        let mode = match status {
            LoopStatus::None => musicyi_core::model::PlayMode::Sequential,
            LoopStatus::Track => musicyi_core::model::PlayMode::Single,
            LoopStatus::Playlist => musicyi_core::model::PlayMode::ListLoop,
        };
        let _ = tx_loop.send(Event::Action(Action::SetMode(mode)));
    });
    let tx_shuffle = tx.clone();
    player.connect_set_shuffle(move |_, shuffle| {
        let _ = tx_shuffle.send(Event::Action(Action::SetShuffle(shuffle)));
    });

    tokio::task::spawn_local(player.run());

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            MprisCmd::Update(state) => {
                let status = if state.stopped {
                    PlaybackStatus::Stopped
                } else if state.playing {
                    PlaybackStatus::Playing
                } else {
                    PlaybackStatus::Paused
                };
                let _ = player.set_playback_status(status).await;
                let _ = player.set_metadata(metadata_for(&state)).await;
                player.set_position(Time::from_millis(state.position_ms as i64));
                let _ = player.set_volume(state.volume.clamp(0.0, 1.0) as f64).await;
                let _ = player.set_loop_status(state.loop_status.into()).await;
                let _ = player.set_shuffle(state.shuffle).await;
                let _ = player.set_can_go_next(state.can_next).await;
                let _ = player.set_can_go_previous(state.can_prev).await;
            }
            MprisCmd::Seeked(ms) => {
                let _ = player.seeked(Time::from_millis(ms as i64)).await;
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn metadata_for(state: &MprisState) -> Metadata {
    if state.title.is_empty() {
        let mut metadata = Metadata::new();
        metadata.set_trackid(Some(TrackId::NO_TRACK));
        return metadata;
    }
    let mut hasher = DefaultHasher::new();
    state.title.hash(&mut hasher);
    state.album.hash(&mut hasher);
    state.artists.hash(&mut hasher);
    let path = format!("/com/musicyi/track/{:016x}", hasher.finish());
    let track = TrackId::try_from(path.as_str()).unwrap_or(TrackId::NO_TRACK);

    let mut builder = Metadata::builder()
        .trackid(track)
        .title(state.title.clone())
        .length(Time::from_millis(state.duration_ms as i64));
    if !state.artists.is_empty() {
        builder = builder.artist(state.artists.clone());
    }
    if !state.album.trim().is_empty() {
        builder = builder.album(state.album.clone());
    }
    if let Some(url) = &state.art_url {
        builder = builder.art_url(url.clone());
    }
    builder.build()
}
