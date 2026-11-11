use std::fs::File;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use rodio::stream::DeviceSinkBuilder;
use rodio::{Decoder, Player};

#[derive(Debug, Clone)]
pub enum Cmd {
    Load {
        path: PathBuf,
        duration_ms: u64,
        start_ms: u64,
        autoplay: bool,
    },
    Play,
    Pause,
    Toggle,
    Stop,
    Seek(Duration),
    Volume(f32),
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    Ready,
    Ended,
    Error(String),
    Position {
        pos_ms: u64,
        dur_ms: u64,
        playing: bool,
    },
}

/// 音频引擎句柄（线程安全，命令走 channel）。
pub struct AudioEngine {
    cmd_tx: Sender<Cmd>,
    event_rx: Receiver<PlayerEvent>,
}

impl AudioEngine {
    pub fn spawn(volume: f32) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ev_tx, event_rx) = mpsc::channel();
        thread::Builder::new()
            .name("musicyi-audio".into())
            .spawn(move || run(cmd_rx, ev_tx, volume))
            .expect("spawn audio thread");
        Self { cmd_tx, event_rx }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn try_recv(&self) -> Option<PlayerEvent> {
        self.event_rx.try_recv().ok()
    }
}

fn run(cmd_rx: Receiver<Cmd>, ev_tx: Sender<PlayerEvent>, initial_volume: f32) {
    let mut sink = match DeviceSinkBuilder::open_default_sink() {
        Ok(s) => s,
        Err(e) => {
            let _ = ev_tx.send(PlayerEvent::Error(format!("打开音频设备失败: {e}")));
            return;
        }
    };
    sink.log_on_drop(false);
    let player = Player::connect_new(sink.mixer());
    player.set_volume(initial_volume);

    let mut loaded = false;
    let mut duration_ms = 0u64;
    let mut ended_sent = true;
    let mut playing = false;
    let mut last_tick = Instant::now();

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                Cmd::Load {
                    path,
                    duration_ms: dur,
                    start_ms,
                    autoplay,
                } => {
                    let file = match File::open(&path) {
                        Ok(f) => f,
                        Err(e) => {
                            let _ = ev_tx.send(PlayerEvent::Error(format!(
                                "打开缓存文件失败 {}: {e}",
                                path.display()
                            )));
                            continue;
                        }
                    };
                    let hint = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("mp3")
                        .to_string();
                    let byte_len = file.metadata().ok().map(|m| m.len()).unwrap_or(0);
                    let decoder = match Decoder::builder()
                        .with_data(file)
                        .with_byte_len(byte_len)
                        .with_seekable(true)
                        .with_hint(&hint)
                        .with_gapless(true)
                        .build()
                    {
                        Ok(d) => d,
                        Err(e) => {
                            let _ = ev_tx.send(PlayerEvent::Error(format!("解码失败: {e}")));
                            continue;
                        }
                    };
                    player.clear();
                    player.append(decoder);
                    duration_ms = dur;
                    loaded = true;
                    ended_sent = false;
                    playing = autoplay;
                    if autoplay {
                        player.play();
                    } else {
                        player.pause();
                    }
                    if start_ms > 0 {
                        let _ = player.try_seek(Duration::from_millis(start_ms));
                    }
                    let _ = ev_tx.send(PlayerEvent::Ready);
                }
                Cmd::Play => {
                    player.play();
                    playing = true;
                    if loaded && ended_sent {
                        ended_sent = false;
                    }
                }
                Cmd::Pause => {
                    player.pause();
                    playing = false;
                }
                Cmd::Toggle => {
                    if player.is_paused() {
                        player.play();
                        playing = true;
                    } else {
                        player.pause();
                        playing = false;
                    }
                }
                Cmd::Stop => {
                    player.clear();
                    loaded = false;
                    playing = false;
                    ended_sent = true;
                    duration_ms = 0;
                }
                Cmd::Seek(pos) => {
                    if let Err(e) = player.try_seek(pos) {
                        tracing::warn!("seek 失败: {e}");
                    }
                }
                Cmd::Volume(v) => player.set_volume(v.clamp(0.0, 2.0)),
            }
        }

        if loaded && !ended_sent && player.empty() {
            ended_sent = true;
            playing = false;
            let _ = ev_tx.send(PlayerEvent::Ended);
        }

        if loaded && last_tick.elapsed() >= Duration::from_millis(200) {
            last_tick = Instant::now();
            let pos = player.get_pos().as_millis() as u64;
            let _ = ev_tx.send(PlayerEvent::Position {
                pos_ms: pos.min(duration_ms.max(pos)),
                dur_ms: duration_ms,
                playing,
            });
        }

        thread::sleep(Duration::from_millis(40));
    }
}
