use anyhow::Context;
use clap::Parser;
use musicyi_core::model::Quality;
use musicyi_core::{Config, Paths};

#[derive(Parser, Debug)]
#[command(name = "musicyi", version, about = "终端里的多来源音乐播放器")]
struct Cli {
    /// 网易云链接 / 歌曲或歌单 ID / M3U 路径 / 搜索关键词
    input: Option<String>,
    /// netease-cloud-music-api 地址，例如 http://127.0.0.1:3000
    #[arg(long)]
    api_base: Option<String>,
    /// 音质：standard / higher / exhigh / lossless / hires
    #[arg(long)]
    quality: Option<String>,
    /// 启动前清空音频缓存
    #[arg(long, default_value_t = false)]
    clear_cache: bool,
    /// 只打印路径后退出
    #[arg(long, default_value_t = false)]
    paths: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let paths = Paths::discover()?;
    paths.ensure()?;

    if cli.paths {
        println!("config:    {}", paths.config_file.display());
        println!("data:      {}", paths.data_dir.display());
        println!("history:   {}", paths.history_file.display());
        println!("session:   {}", paths.session_file.display());
        println!("playlists: {}", paths.playlists_dir.display());
        println!("cache:     {}", paths.audio_cache.display());
        println!("logs:      {}", paths.log_dir.display());
        return Ok(());
    }

    init_tracing(&paths)?;

    let mut cfg = Config::load_or_create(&paths)?;
    if let Some(base) = cli.api_base {
        cfg.api.base_url = base;
    }
    if let Some(q) = cli.quality {
        cfg.player.quality = Quality::from_level(&q);
    }
    if cli.clear_cache
        && let Ok(read) = std::fs::read_dir(&paths.audio_cache)
    {
        for entry in read.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("musicyi")
        .build()
        .context("创建 tokio runtime 失败")?;

    let result = rt.block_on(musicyi_tui::run(cfg, paths.clone(), cli.input));
    if let Err(e) = &result {
        tracing::error!("主循环退出: {e}");
    }
    result.context("TUI 运行失败")?;
    Ok(())
}

fn init_tracing(paths: &Paths) -> anyhow::Result<()> {
    std::fs::create_dir_all(&paths.log_dir).ok();
    let file = tracing_appender::rolling::daily(&paths.log_dir, "musicyi.log");
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_writer(file)
        .with_ansi(false)
        .with_env_filter(filter)
        .init();
    Ok(())
}
