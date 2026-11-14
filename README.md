# musicyi

[![CI](https://github.com/yifedora/musicyi/actions/workflows/ci.yml/badge.svg)](https://github.com/yifedora/musicyi/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/yifedora/musicyi)](https://github.com/yifedora/musicyi/releases)
[![License: GPL v3](https://img.shields.io/badge/License-GPL_v3-blue.svg)](LICENSE)

终端里的多来源音乐播放器。单一首页五个入口（网易原创 / 街声推荐 / 雀乐推荐 / 本地列表 / 最近播放），`Esc` 逐级返回并停在上次位置，交互灵感来自 [go-musicfox](https://github.com/go-musicfox/go-musicfox)。

![musicyi 首页](docs/screenshot.png)

| 歌词纯享 | 雀乐推荐列表 |
| --- | --- |
| ![歌词纯享](docs/screenshot-lyrics.png) | ![雀乐推荐列表](docs/screenshot-indie.png) |

## 功能

- **网易云**：只用官方直链与官方 30 秒试听，不经过任何解灰音源；无法播放自动跳过。
  保留单曲搜索、网易云歌单搜索与链接 / ID / M3U 打开。
- **街声 / 雀乐**：本机 API 转发（街声 HLS → MP3；雀乐直接取 CDN MP3），含歌词。
- **本地音乐**：递归扫描目录，文件名 `歌手 - 标题.mp3` 作为主线索；`t` / `T` 转换选中 / 批量转换，
  先查网易云，未命中自动回退 MusicBrainz（元数据 / 封面）与 LRCLIB（歌词），
  繁体曲名先经 OpenCC 转简再匹配，全部失败写入 `tag-failures.log`。
  **转换副本写到独立输出目录，原始文件始终不动**。
- **歌词**：默认关闭，`l` 开 / 关；LRC 自动滚动，也支持雀乐 / 街声的纯文本歌词按宽度换行。
  `v` 进入歌词纯享（歌词铺满内容区）。
- **播放**：`Space` 暂停，`]` / `[` 切歌，`←` / `→` 快进退 5 秒，四种播放模式，
  进度 seek，音量记忆，退出保存队列与进度（下次启动暂停在原位置）。
- **下载**：`d` 下载选中歌曲到 `[download] dir`（默认 `~/Music/musicyi`），
  文件名为 `歌手 - 标题.扩展名`，同名自动追加序号；下载后自动写入标题 / 歌手 / 专辑、
  歌词（USLT）与封面（Front Cover），一次只下载一首。
- **MPRIS**：媒体键 / `playerctl` / 桌面媒体控件（仅 Linux）。

## 环境要求

- Linux（PipeWire 或 PulseAudio）
- Rust 1.88+
- Node.js 18+（本机网易云 API）
- `ffmpeg`（街声 HLS 转 MP3，需在 `PATH` 中）
- 编译依赖：Fedora `alsa-lib-devel pkg-config`；Debian / Ubuntu `libasound2-dev pkg-config`

## 快速开始

```bash
git clone https://github.com/yifedora/musicyi.git
cd musicyi
./install.sh                    # 构建 release 并安装到 ~/.local/bin

cd api-server && npm ci && cd ..
./api-server/start.sh           # 启动本机 API（默认 127.0.0.1:3000）
musicyi
```

关闭 API：`./api-server/stop.sh`。

## 基本用法

```bash
musicyi                                  # 首页
musicyi "关键词"                          # 搜索单曲
musicyi "https://music.163.com/#/song?id=1858110236"
musicyi "https://music.163.com/#/playlist?id=3778678"
musicyi ~/Music/list.m3u                 # 打开 M3U（可含路径 / 链接 / ID）
musicyi ~/Music/song.flac                # 播放本地文件
```

首页 `Enter` 进入栏目，再 `Enter` 播放，`Esc` 返回。搜索 / 歌单 / 打开分别按 `/`、`f`、`o`，
输入框内 `Enter` 确认，`Esc` 放弃。

## 快捷键

| 按键 | 作用 |
| --- | --- |
| `j` / `k`、`↓` / `↑` | 上下移动 |
| `g` / `G` | 跳到列表首 / 尾 |
| `Enter` | 打开栏目 / 播放选中歌曲 / 确认输入 |
| `Esc` | 收起输入 / 返回上一级 / 回到首页 |
| `/` / `f` / `o` | 单曲搜索 / 歌单搜索 / 打开路径·链接·ID·M3U |
| `Space` | 播放 / 暂停 |
| `]` / `[` | 下一首 / 上一首 |
| `←` / `→` | 快退 / 快进 5 秒 |
| `+` / `-` | 音量加 / 减 |
| `r` | 切换播放模式（列表循环 / 单曲 / 随机 / 顺序） |
| `l` | 歌词开 / 关（默认关） |
| `v` | 歌词纯享（进入强制开歌词） |
| `PageUp` / `PageDown` | 滚动歌词 |
| `t` / `T` | 转换并补全选中 / 批量补全未转换的本地歌曲 |
| `x` / `Delete` | 从最近播放删除选中记录 |
| `d` | 下载选中歌曲（自动写入标题/歌手/专辑/歌词/封面） |
| `R` | 刷新当前栏目 |
| `?` | 打开 / 关闭快捷键帮助 |
| `q` | 退出（`Ctrl+C` 强制退出） |

输入状态下 `Enter` 确认、`Esc` 放弃，其余按键按文本处理。**`F` 跟随开关已移除**：
自动切歌始终把指针移到正在播放的歌曲。

## 配置

配置文件首次启动自动生成，位于 `~/.config/musicyi/config.toml`，常用项：

```toml
[player]
quality = "exhigh"   # standard / higher / exhigh / lossless / hires
volume = 0.8
mode = "list_loop"   # list_loop / single / shuffle / sequential

[local]
dir = ""             # 留空用 ~/Music/playlist
output_dir = ""      # 留空用 ~/Music/musicyi/local
tag_delay_ms = 220   # 批量补全间隔，避免风控

[ui]
mpris = true
```

设置 `MUSICYI_ROOT` 可把所有可写路径收拢到指定目录，便于便携使用。完整配置项、
默认路径、`musicyi --paths` 输出见 [DEVELOPMENT.md](DEVELOPMENT.md)。

## 已知限制

- **本地标签补全无法覆盖正版下架 / 仅特定平台有收录**（如诱导社《美好时代》太声文化版、
  无边界音乐《云南来信》QQ 音乐版）的曲目；这类曲目会写入 `tag-failures.log`，
  人工补充别名或链接即可。
- **网易云对英文主标题**（The Faith 最终信仰 / Forgotten River）几乎搜不到；
  目前依赖内嵌标签里的中文名或保留 `中文 - 英文脚注` 中中文部分兜底。
- **街声 / 雀乐** 依赖本机 API 转发，源站接口变动会立即失效，需要更新 api-server。

## 常见问题

**启动后显示“API 未连接”**

```bash
./api-server/start.sh
tail -n 50 api-server/server.log
```

节点首次启动需联网获取匿名 token，稍等几秒。

**接口返回 `-460` 风控错误**

网易拒绝了当前出口 IP，在 `config.toml` 填入可用 IP：

```toml
[api]
real_ip = "114.114.114.114"
```

只随 `song/url/v1`（取播放链接）发送，不影响搜索。

**本地转换没有生成副本**

先确认 API 可用与 `output_dir` 可写（与 `dir` 相同会被拒绝）。失败曲目会追加到
`<日志目录>/tag-failures.log`，首行即原因，可人工核查。

**M3U 里的本地文件打不开**

确认路径存在、扩展名受 rodio / symphonia 支持（mp3 / flac / m4a / aac / ogg / opus / wav / aiff / alac）。

**歌词为空**

远程歌曲可能没有歌词；翻译取决于接口是否返回。本地歌曲读取转换副本里的内嵌歌词（USLT）。

**想重新开始**

```bash
rm -rf ~/.cache/musicyi/audio \
       ~/.local/share/musicyi/history.jsonl \
       ~/.local/share/musicyi/session.json
```

下载与转换后的音乐文件不会被删除。

## 版权说明

- 本软件是播放器，不拥有、不存储、不分发任何音乐作品；歌曲、封面、歌词、专辑信息的版权归原权利人所有。
- **仅限互联网试听**：通过音源平台获取的音频仅用于个人试听；网易云等只能拿到官方 30 秒试听片段，即为正常现象。
- **24 小时内删除**：试听缓存（`~/.cache/musicyi/audio/`）及通过 `d` 下载的文件，均须在 **24 小时内删除**；
  禁止传播、上传、出售或用于商业用途。
- 本项目与各音源平台无任何隶属或背书关系；因使用本软件产生的版权纠纷由使用者自行承担。
- 若权利人认为本项目侵犯其权益，请通过 GitHub Issues 联系，我们会及时处理。

## 许可

版权所有 © 2026 yifedora。本软件以 GPL-3.0-or-later 发布，见 [LICENSE](LICENSE)。

接口能力来自开源项目 [netease-cloud-music-api-alger](https://github.com/algerkong/netease-cloud-music-api)，
交互灵感来自 [go-musicfox](https://github.com/go-musicfox/go-musicfox) 与
[AlgerMusicPlayer](https://github.com/algerkong/AlgerMusicPlayer)。

开发者文档（架构 / 目录结构 / api-server / CI / 发布流程 / 完整配置项）见 [DEVELOPMENT.md](DEVELOPMENT.md)。
