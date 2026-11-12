pub mod app;
pub mod event;
mod fallback;
pub mod history;
pub mod input;
pub mod m3u;
pub mod mpris;
pub mod theme;
pub mod types;
pub mod ui;

use std::time::Duration;

use musicyi_core::Paths;
use musicyi_core::config::Config;
use ratatui::crossterm::event as ct;

pub use app::App;

/// 启动 TUI 主循环。返回后终端已恢复。
pub async fn run(cfg: Config, paths: Paths, initial: Option<String>) -> std::io::Result<()> {
    let (mut app, mut rx) = App::new(cfg, paths, initial).map_err(std::io::Error::other)?;

    let mut terminal = ratatui::init();

    let tx = app.sender();
    std::thread::Builder::new()
        .name("musicyi-input".into())
        .spawn(move || {
            loop {
                match ct::read() {
                    Ok(ct::Event::Key(k)) => {
                        if tx.send(event::Event::Key(k)).is_err() {
                            break;
                        }
                    }
                    Ok(ct::Event::Resize(_, _)) => {
                        if tx.send(event::Event::Resize).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        })
        .ok();

    let mut ticker = tokio::time::interval(Duration::from_millis(100));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut result = Ok(());
    loop {
        if let Err(e) = terminal.draw(|frame| app.render(frame)) {
            result = Err(e);
            break;
        }
        tokio::select! {
            maybe = rx.recv() => {
                if let Some(ev) = maybe {
                    app.handle_event(ev);
                }
            }
            _ = ticker.tick() => {
                app.tick();
            }
        }
        if app.quit() {
            break;
        }
    }

    drop(terminal);
    ratatui::restore();
    app.finish();
    result
}
