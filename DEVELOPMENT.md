# DEVELOPMENT

面向开发者的文档：架构、目录结构、api-server 内幕、CI 与发布流程。
用户使用文档见 [README.md](README.md)。

## 架构

- **分层**（从内到外的依赖方向）：
  `musicyi-core`（模型 / 配置 / 路径 / LRC 与纯文本歌词解析 / 错误类型）
  → `musicyi-api`（网易云接口客户端、来源 DTO、官方音源解析）
  → `musicyi-player`（rodio 播放线程、音频缓存与 seek）
  → `musicyi-library`（lofty 标签 / 歌词 / 封面读写、下载落盘）
  → `musicyi-tui`（ratatui 单一首页界面、事件循环、MPRIS）
  → `musicyi`（可执行入口：CLI 参数、日志、tokio runtime）
- **事件流**：crossterm 键鼠事件与异步任务回调统一经 unbounded channel 汇入主循环
  （tokio current_thread runtime）；播放在独立线程（rodio）运行，通过 channel 回报
  进度 / 播完 / 错误。
- **MPRIS**：`mpris-server::Player` 内部使用 `Rc`，不是 `Send`，因此在单独线程中运行
  （current_thread runtime + LocalSet），与 TUI 通过 channel 通信；仅 Linux 编译
  （`cfg(target_os = "linux")`，其余平台为空实现），功能开关为 `[ui] mpris`。
- **路径约定**：全部可写路径集中在 `musicyi_core::paths::Paths`，遵循 XDG
  （`~/.config/musicyi`、`~/.local/share/musicyi`、`~/.cache/musicyi`、
  `~/.local/state/musicyi/logs`）；设置 `MUSICYI_ROOT` 后收拢到该目录下，便于便携使用与测试。
- **存储**：M3U 歌单 + JSONL 播放历史 + TOML 配置；不引入数据库。
- **缓存**：音频先落到 `~/.cache/musicyi/audio` 再解码，seek 准确；可用
  `musicyi --clear-cache` 清空，删除源码目录不影响缓存。

## 目录结构

```
musicyi/
├── Cargo.toml                 # workspace（统一 version / license / rust-version / repository / homepage）
├── LICENSE                    # GPL-3.0 官方正文
├── README.md                  # 用户向文档
├── DEVELOPMENT.md             # 本文件
├── docs/
│   ├── screenshot.png         # README 首页截图
│   ├── screenshot-lyrics.png  # README 歌词纯享截图
│   └── screenshot-indie.png   # README 雀乐推荐列表截图
├── .github/workflows/ci.yml   # CI：fmt / check / test / clippy
├── install.sh                 # 构建 release 并安装到 ~/.local/bin（--uninstall 只删二进制）
├── api-server/                # Node 网易云 API + 雀乐 / 街声目录（npm，node_modules 不入库）
│   ├── index.js               # 手写启动器：实例化 netease-cloud-music-api，挂 /musicyi/health 与 musicyi 路由
│   ├── indie.js               # 雀乐最新 10 期
│   ├── streetvoice.js         # 街声推荐与 HLS → MP3
│   ├── start.sh / stop.sh     # bash 启停（端口检测用 ss -ltnp，HOST 默认 127.0.0.1）
│   ├── package.json           # 依赖锁定声明（netease-cloud-music-api-alger@^4.32.0）
│   └── package-lock.json      # 精确锁定，`npm ci` 可复现安装
└── crates/
    ├── musicyi/               # 可执行入口（CLI / 日志 / runtime）
    ├── musicyi-core/          # 模型、配置、路径、LRC / 纯文本解析
    ├── musicyi-api/           # 网易云接口、来源 DTO、官方音源解析
    ├── musicyi-player/        # rodio 播放线程、音频缓存
    ├── musicyi-library/       # lofty 标签 / 歌词 / 封面读写、下载落盘
    └── musicyi-tui/           # 单一首页 ratatui 界面、事件循环、MPRIS、外部标签回退
        └── src/fallback.rs    # 网易云未命中时：MusicBrainz（元数据/CAA 封面）+ LRCLIB（歌词）回退；
                               # tag-failures.log 落盘（log_dir 下 TSV：时间/文件/query/原因）
```

## api-server

`api-server/` 是运行时硬依赖（提供播放链接与目录数据），但不作为常驻服务：
`./api-server/start.sh` 手动启动，默认只监听 `127.0.0.1`，不上 systemd。

手写启动器做的事（`index.js`）：

1. 实例化 `netease-cloud-music-api-alger`，获得 weapi/eapi 加密、匿名 token、登录态与风控处理能力；
2. 在同一个 Express 实例上挂 musicyi 的自有路由；
3. 暴露 `GET /musicyi/health` 作为健康检查。

```text
GET  /musicyi/health                                  → {"ok":true,"catalog":true,"stream":true}
GET  /musicyi/catalog/indie                           雀乐最新 10 期：歌单 + 歌曲同时返回
GET  /musicyi/catalog/streetvoice/realtime            街声实时榜
GET  /musicyi/catalog/streetvoice/recommend           街声推荐前 30（每页 20，自动翻页去重）
GET  /musicyi/catalog/streetvoice/:id                 单曲详情（时长 / 专辑 / 封面 / 歌词）
GET  /musicyi/stream/streetvoice/:id                  播放：取 HLS 地址 → ffmpeg 无损转封装为 MP3
```

要点：

- **雀乐**：抓 `indie.cn` 首页最新 10 个 `vol`，播放地址、封面、歌词均直接使用
  `cdn2.indie.cn` 的链接；歌词是无时间轴纯文本时按纯文本显示。
- **街声**：推荐列表为匿名抓取；播放时 `POST /api/v5/song/{id}/hls/file/` 拿 HLS 地址，
  再由 `ffmpeg` 转封装（`-c copy`）为 MP3 缓存，不转码、不降质量。
- **端口**：3000；`start.sh` 已在运行时直接返回，可重复执行。`HOST=0.0.0.0` 才暴露到局域网。
- **依赖**：`node_modules` 不入库，换机器后 `npm ci` 即可（`package-lock.json` 保证可复现）。
- **健康检查**：`curl http://127.0.0.1:3000/musicyi/health`。

### 后端为什么保留 Node

网易接口的 weapi/eapi 加密、匿名 token、登录态与风控都在持续变化。用 Rust 重写
全部接口成本高、维护风险大；当前保留本机 Node API 作为唯一后端，客户端只做重试、
分页与缓存。需要播放时先运行 `./api-server/start.sh`。

## CI

`.github/workflows/ci.yml`，push / PR 触发，单个 job：

1. **fmt / check / test / clippy**（`ubuntu-latest`）：apt 装 `libasound2-dev pkg-config` →
   `cargo fmt --all --check` → `cargo check --workspace --all-targets` →
   `cargo test --workspace` → `cargo clippy --workspace --all-targets -- -D warnings`

Windows `cfg` 门控漏网之鱼不再有兜底 job（曾因 runner 分配失败长期无法运行，已移除）。
修改 Unix-only 代码时注意手动跑 `cargo check --target x86_64-pc-windows-msvc`，或自行加门槛。

已知仅警告、不影响结果：`actions/checkout@v4` 被强制跑在 Node 24；
`ubuntu-latest` 于 2026-10 起迁移 Ubuntu 26。

## 本地开发

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`./install.sh`：从源码构建 release 并安装到 `~/.local/bin`（`PREFIX` 可覆盖）；
会检查 ALSA 开发库（`pkg-config --exists alsa`）；`--uninstall` 只删除二进制，
不触碰 `~/.config/musicyi` 与 `~/.local/share/musicyi` 中的用户数据。

国内网络构建慢时，在**自己的** `~/.cargo/config.toml` 配置源替换，例如：

```toml
[source.crates-io]
replace-with = "rsproxy-sparse"
[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"
```

或者：`export https_proxy=http://127.0.0.1:7897` 后直连 crates.io。
仓库不内置任何镜像配置（分发给所有用户，第三方镜像不应强制生效）。

## 发布流程

1. 修改 `Cargo.toml` `[workspace.package]` 的 `version`；
2. `cargo fmt / clippy / test` 全绿（CI 双 job 也必须绿）；
3. `git commit` → `git tag -a vX.Y.Z -m "vX.Y.Z"` → `git push && git push --tags`；
4. 发布正式版 Release：

   ```bash
   gh release create vX.Y.Z --title "musicyi vX.Y.Z" --notes-file release-notes.md
   ```

5. 不随包分发二进制：用户走源码 `./install.sh`（或 `cargo build --release`）自行构建；
   git tag 与 `vX.Y.Z` Release 保持一一对应。

crate 元数据统一在 workspace 声明（`description` / `readme` / `repository` / `homepage` /
`license` / `rust-version`），各 crate 以 `*.workspace = true` 继承；
仅 `description` 各自不同。

## 本地标签补全（t / T）

流程在 `crates/musicyi-tui/src/app.rs`：`resolve_and_convert()` 先走网易云
`cloudsearch`，`pick_best_match()` 按标题 / 歌手 / 时长（容差 10s）取最高分；繁体
曲名先经 `ferrous-opencc`（`T2s`，内置于二进制，无系统依赖）归一到简体再参加匹配。
未命中或转换失败时，`prepare_fallback()` 改查 MusicBrainz（CC0；title/artist 归一
包含匹配 + 时长校验；封面走 Cover Art Archive `front-500`），歌词走
`lrclib_lyrics()`（`/api/search`，`syncedLyrics` 优先，其次 `plainLyrics`；
LRCLIB 中文覆盖抽样较差，会返回空）。全部失败追加写
`<日志目录>/tag-failures.log`（TSV）。注意：MusicBrainz 官方限速 1 req/s，当前
实现未做限速（批量请求可能触发节流）；LRCLIB 无 key 无限速。两条请求路径都带独立
超时且不会 panic。

## 已知问题 / 技术债

- `crates/musicyi-tui/src/app.rs` 约 3400 行，80 个方法的 god object：`render()`（~250 行）、
  键盘路由（~600 行）、播放队列（~400 行）、数据加载（~500 行）是已知可安全拆分的四块；
  计划在 v0.1.0 之后的迭代中分多次 Pr 完成。
- TUI 渲染层只有少量辅助函数有测试；布局类函数（如 `truncate_width`）有覆盖，其余依赖 tmux 人工验证。
- MPRIS 仅 Linux；Windows 支持在规划中（`start.ps1` / `stop.ps1` / `install.ps1`）。

## 依赖与许可

- 全仓库 GPL-3.0-or-later；[LICENSE](LICENSE) 为官方 GPL-3.0 正文。
- 主要依赖：ratatui / crossterm（TUI）、tokio + reqwest（rustls）、rodio + symphonia（音频解码，
  纯 Rust）、lofty（标签）、dirs、clap、rand、tracing、mpris-server（仅 Linux）、
  ferrous-opencc（繁→简，内嵌字典，不依赖系统 OpenCC）；
  运行时另需 Node 18+ 的 `netease-cloud-music-api-alger`（MIT）与 `ffmpeg`（街声 HLS 转封装）。
- 交互灵感来自 [go-musicfox](https://github.com/go-musicfox/go-musicfox) 与
  [AlgerMusicPlayer](https://github.com/algerkong/AlgerMusicPlayer)（鸣谢，未衍生代码）。

## 完整默认路径与配置项

XDG 默认路径：

- 配置：`~/.config/musicyi/config.toml`
- 数据：`~/.local/share/musicyi/`
  - 历史：`history.jsonl`
  - 会话：`session.json`
  - 歌单目录：`playlists/`（保留给 M3U 文件，程序不会自动扫描）
- 本地音乐默认目录：`~/Music/playlist/`
- 本地转换默认目录：`~/Music/musicyi/local/`
- 音频缓存：`~/.cache/musicyi/audio/`
- 日志：`~/.local/state/musicyi/logs/`

`config.toml` 全部常用项：

```toml
[api]
base_url = "http://127.0.0.1:3000"
timeout_secs = 15
real_ip = ""                      # 需要时填，绕开部分网络风控；只随 song/url/v1 发送

[player]
quality = "exhigh"                # standard / higher / exhigh / lossless / hires
volume = 0.8
mode = "list_loop"                # list_loop / single / shuffle / sequential
cache_limit_mb = 2048

[ui]
restore_session = true            # 退出保存队列 / 索引 / 进度
mpris = true                      # MPRIS 桌面控制

[download]
dir = ""                          # 留空使用 ~/Music/musicyi

[local]
dir = ""                          # 留空使用 ~/Music/playlist
output_dir = ""                   # 留空使用 ~/Music/musicyi/local
tag_delay_ms = 220                # 批量转换间隔，避免风控
```

旧配置里的 `[source]` 表（`unblock` / `unblock_order` / `proxy` / `custom`）会被自动忽略，
保存配置时消失。`MUSICYI_ROOT` 环境变量可把所有可写路径收拢到指定目录用于便携使用与测试。

命令行参数：

```bash
musicyi [URL|ID|M3U|关键词]
musicyi --api-base http://127.0.0.1:3000
musicyi --quality lossless
musicyi --clear-cache
musicyi --paths                  # 打印配置 / 数据 / 缓存 / 日志路径
```

> 旧版 SQLite 已不再读写，可直接删除：`rm -f ~/.local/share/musicyi/musicyi.db*`。
> 历史会从新的 JSONL 文件重新积累。

## 本地转换与标签（详）

- 转换依赖网易云搜索；找不到匹配即跳过，不会写入猜测结果。
- 输出目录与源目录相同时拒绝转换，避免覆盖原文件。
- 转换副本按原目录结构保存，扩展名不变，原文件不动；列表优先播放副本。
- 找不到匹配 / 网络失败 / 目标目录不可写时会追加写 `<日志目录>/tag-failures.log`
  （TSV：时间 / 文件路径 / query / 原因），默认日志目录见 `musicyi --paths`。
- 转换副本含 title / artist / album、歌词（USLT）与封面（Front Cover），均经 lofty 写入。
- 雀乐 / 街声的纯文本歌词按宽度换行；LRC 自动滚动，`PageUp` / `PageDown` 手动滚动。
- `d` 下载：文件名 `{artist} - {title}.{ext}`，同名自动追加 ` (1)`、` (2)`，
  不写标签也不嵌封面 / 歌词，一次只下载一首。

## 音源策略

1. **网易官方直链**：`/song/url/v1`，按当前音质请求；拿不到 URL 时自动逐档降到
   `standard`，避免无损不可用导致整首失败。
2. **官方试听**：只有官方返回 30 秒试听时才播放试听，不再尝试任何第三方音源。
3. **无法播放**：自动跳到下一首；连续失败达到队列长度（最多 8 首）后停止。

## 雀乐与街声

- **雀乐**：本机 API 抓 `indie.cn` 首页最新 10 个 `vol`，汇总期数歌单与歌曲；
  播放地址、封面、歌词都直接使用 `cdn2.indie.cn` 的链接；无时间轴纯文本歌词按纯文本显示。
- **街声**：推荐前 30 首（每页 20 首，自动翻页去重），详情接口补时长 / 专辑 / 封面 / 歌词；
  播放时本机 API 把 HLS 无损转封装为 MP3 缓存。

## 常见问题（开发者向）

**MPRIS 没有出现**：确认运行在图形会话且 `DBUS_SESSION_BUS_ADDRESS` 可用、
`[ui].mpris = true`；重启后 `playerctl -l` 应列出 `musicyi`（D-Bus 名称
`org.mpris.MediaPlayer2.musicyi`）。

**foot 右侧比左侧留白远**：窗口宽度不是字符宽度整数倍时，foot 默认把多余像素放右侧。
`~/.config/foot/foot.ini` 的 `[main]` 加 `pad=0x0 center` 让网格居中。
