use musicyi_core::lyric::Lyrics;
use musicyi_core::model::{PlayMode, Playlist, Song, SourceKind};
use musicyi_core::util::{format_duration, human_count, pad_width, truncate_width};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, HighlightSpacing, Paragraph, Row, Table, TableState,
    Wrap,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::Theme;
use crate::types::{CoverArt, Focus, InputMode};

/// 窗口低于最小尺寸时只显示这一行，避免各面板挤在一起。
pub fn render_too_small(frame: &mut Frame, area: Rect, _theme: &Theme) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let message = truncate_width("终端太小，请放大窗口或按 l 隐藏歌词", area.width as usize);
    let line = Line::from(Span::raw(message)).alignment(Alignment::Center);
    frame.render_widget(
        Paragraph::new(line),
        Rect {
            y: area.y + (area.height / 2).saturating_sub(1),
            height: 2,
            ..area
        },
    );
}

/// fox 风格的圆角弹窗：macOS Toast 一样的边框 + 强调色标题。
/// 目前用于帮助浮层；歌词/列表主页不再使用边框。
/// 帮助浮层：圆角边框 + 默认样式标题，不再使用强调色。
pub fn block(title: &str, _theme: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::raw(format!(" {title} ")))
}

/// 首页字标：4 像素高（2 个终端行）的「MUSICYI」块字，品牌红。
const BRAND_ART: [&str; 2] = [
    "█▄ ▄█ █   █ ▄▀▀▀▀ ▀█▀ ▄▀▀▀▀ ▀▄ ▄▀ ▀█▀",
    "█ ▀ █ █▄▄▄█ ▄▄▄▄▀ ▄█▄ ▀▄▄▄▄   █   ▄█▄",
];

/// 首页入口列表。
pub fn render_home(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    items: &[&str],
    state: &mut TableState,
    _title: &str,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    if items.is_empty() {
        draw_message(frame, area, "栏目加载中…", Style::default());
        return;
    }

    let selected = state.selected();
    // 垂直居中：顶部一行空行 + 两行字标 + 一行空隙 + 单列菜单。
    let menu_h = items.len() as u16;
    let content_h = 1 + BRAND_ART.len() as u16 + 1 + menu_h;
    let top_pad = area.height.saturating_sub(content_h) / 2;

    let rows = Layout::vertical([
        Constraint::Length(top_pad),
        Constraint::Length(1), // 顶部空行
        Constraint::Length(BRAND_ART.len() as u16),
        Constraint::Length(1), // gap
        Constraint::Length(menu_h),
        Constraint::Min(0),
    ])
    .split(area);

    let art_lines = BRAND_ART.iter().map(|line| {
        Line::from(Span::styled(
            line.to_string(),
            Style::default()
                .fg(theme.bender)
                .add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center)
    });
    frame.render_widget(Paragraph::new(art_lines.collect::<Vec<_>>()), rows[2]);

    // 单列居中菜单：整块按最长一行宽度居中，文字左对齐；
    // 选中行整行使用暗色 pill 背景 + 品牌红粗体。
    let items_with_num: Vec<String> = items
        .iter()
        .enumerate()
        .map(|(i, label)| format!("{}.  {}", i + 1, label))
        .collect();
    let text_w = items_with_num
        .iter()
        .map(|text| UnicodeWidthStr::width(text.as_str()))
        .max()
        .unwrap_or(0)
        .min(area.width as usize);
    let block_x = area.x + ((area.width as usize).saturating_sub(text_w) / 2) as u16;
    let block_area = Rect {
        x: block_x,
        y: rows[4].y,
        width: text_w as u16,
        height: menu_h.min(rows[4].height),
    };
    let menu_lines = items_with_num
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let is_selected = selected == Some(index);
            let style = if is_selected {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let padded = pad_width(&truncate_width(text, text_w).to_string(), text_w);
            Line::from(Span::styled(padded, style))
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(menu_lines), block_area);
}

/// 空列表 / 空歌单的占位文案：加载中显示"加载中…"，否则显示传入的提示。
fn empty_state_message(loading: bool, empty: &str) -> &str {
    if loading { "加载中…" } else { empty }
}

fn draw_message(frame: &mut Frame, area: Rect, message: &str, style: Style) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let rect = Rect {
        y: area.y + area.height / 2,
        height: 1,
        ..area
    };
    let line = Line::from(Span::styled(message.to_string(), style)).alignment(Alignment::Center);
    frame.render_widget(Paragraph::new(line), rect);
}

/// 列表为空时的错误提示，允许换行并保留重试入口。
fn draw_error(frame: &mut Frame, area: Rect, message: &str, _theme: &Theme) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let height = area.height.clamp(1, 5);
    let rect = Rect {
        y: area.y + area.height.saturating_sub(height) / 2,
        height,
        ..area
    };
    let text = format!("{message}\n按 R 重试");
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        rect,
    );
}

// 首页 / 列表标题行：`‹ 标题`，仅加粗，不使用颜色。
fn heading_line(title: &str, _theme: &Theme) -> Line<'static> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    Line::from(vec![
        Span::styled("‹ ", bold),
        Span::styled(title.to_string(), bold),
    ])
}

/// 拆出列表区：第一行是 `‹ 标题`，剩余给表格 / 提示。
fn list_split(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
    (chunks[0], chunks[1])
}

fn song_table<'a>(
    songs: &'a [Song],
    current_key: Option<&str>,
    theme: &Theme,
    width: u16,
    highlight_current: bool,
) -> Table<'a> {
    let symbol_w = 2usize;
    let spacing = 1usize;
    // 序号列按实际条目数计算位数，1000+ 首时不会截断。
    let idx_w = songs.len().to_string().len().max(2);
    let dur_w = songs
        .iter()
        .map(|song| {
            if song.duration_ms > 0 {
                format_duration(song.duration_ms).len()
            } else {
                5
            }
        })
        .max()
        .unwrap_or(5)
        .max(5);
    // 街声没有专辑信息，固定只显示 # / 标题 / 歌手 / 时长；
    // 其他来源在宽度足够（约 96 列以上）时才加入专辑列。
    let has_album = songs.iter().any(|song| !song.album.trim().is_empty())
        && !songs
            .iter()
            .all(|song| song.source == SourceKind::StreetVoice);
    let total = width as usize;
    let base = symbol_w + idx_w + dur_w;
    let mut show_album = false;
    let mut title_w = 0usize;
    let mut artist_w = 0usize;
    let mut album_w = 0usize;
    if has_album {
        let content = total.saturating_sub(base + 4 * spacing);
        let title = (content * 40 / 100).max(28);
        let artist = (content * 22 / 100).max(18);
        let album = content.saturating_sub(title + artist);
        if album >= 31 {
            show_album = true;
            title_w = title;
            artist_w = artist;
            album_w = album;
        }
    }
    if !show_album {
        let content = total.saturating_sub(base + 3 * spacing);
        title_w = ((content * 58 / 100).max(10)).min(content);
        artist_w = content.saturating_sub(title_w);
    }

    let rows = songs.iter().enumerate().map(|(i, song)| {
        let is_current = current_key.is_some_and(|key| key == song.key());
        let title = truncate_width(&song.name, title_w);
        let artist = truncate_width(&song.artist_text(), artist_w);
        let duration = if song.duration_ms > 0 {
            format_duration(song.duration_ms)
        } else {
            "--:--".to_string()
        };
        let mut cells = vec![
            Cell::from(format!("{:>idx_w$}", i + 1)),
            Cell::from(title),
            Cell::from(artist),
        ];
        if show_album {
            let album = if song.album.trim().is_empty() {
                "—".to_string()
            } else {
                truncate_width(&song.album, album_w)
            };
            cells.push(Cell::from(album));
        }
        cells.push(Cell::from(format!("{:>dur_w$}", duration)));
        let row = Row::new(cells).height(1);
        if is_current {
            row.style(Style::default().fg(theme.bender))
        } else {
            row
        }
    });

    let mut widths = vec![
        Constraint::Length(idx_w as u16),
        Constraint::Length(title_w as u16),
        Constraint::Length(artist_w as u16),
    ];
    if show_album {
        widths.push(Constraint::Length(album_w as u16));
    }
    widths.push(Constraint::Length(dur_w as u16));

    // 选中行若恰好是当前播放歌曲，则与 bender 保持一致（红黄中间值）。
    let highlight_fg = if highlight_current {
        theme.bender
    } else {
        theme.accent
    };

    Table::new(rows, widths)
        .column_spacing(spacing as u16)
        .highlight_symbol("  ")
        .highlight_spacing(HighlightSpacing::Always)
        .row_highlight_style(
            Style::default()
                .fg(highlight_fg)
                .add_modifier(Modifier::BOLD),
        )
}

#[allow(clippy::too_many_arguments)]
pub fn render_songs(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    songs: &[Song],
    current_key: Option<&str>,
    state: &mut TableState,
    title: &str,
    loading: bool,
    error: Option<&str>,
    empty: &str,
) {
    // 有数据时错误只占标题，列表继续显示；空列表才整屏显示错误。
    let title = if error.is_some() && !songs.is_empty() {
        format!("{title} · 加载失败")
    } else {
        title.to_string()
    };
    let title = truncate_width(&title, area.width.saturating_sub(2) as usize);
    if area.height < 2 || area.width == 0 {
        return;
    }
    let (heading_area, inner) = list_split(area);
    frame.render_widget(Paragraph::new(heading_line(&title, theme)), heading_area);

    if let Some(err) = error
        && songs.is_empty()
    {
        draw_error(frame, inner, err, theme);
        return;
    }
    if songs.is_empty() {
        draw_message(
            frame,
            inner,
            empty_state_message(loading, empty),
            Style::default(),
        );
        return;
    }
    let selected_is_current = state
        .selected()
        .and_then(|index| songs.get(index))
        .is_some_and(|song| current_key.is_some_and(|key| key == song.key().as_str()));
    let table = song_table(songs, current_key, theme, inner.width, selected_is_current);
    frame.render_stateful_widget(table, inner, state);
}

fn playlist_table<'a>(playlists: &'a [Playlist], theme: &Theme, width: u16) -> Table<'a> {
    let symbol_w = 2usize;
    let spacing = 1usize;
    let idx_w = playlists.len().to_string().len().max(2);
    let track_w = playlists
        .iter()
        .map(|playlist| human_count(playlist.track_count).len())
        .max()
        .unwrap_or(4)
        .max(4);
    let show_creator = width as usize >= 72
        && playlists
            .iter()
            .any(|playlist| !playlist.creator.trim().is_empty());
    let total = width as usize;
    let base = symbol_w + idx_w + track_w;
    let content = total.saturating_sub(base + (if show_creator { 3 } else { 2 }) * spacing);
    let (name_w, creator_w) = if show_creator {
        let name = ((content * 62 / 100).max(12)).min(content);
        (name, content.saturating_sub(name))
    } else {
        (content, 0)
    };

    let rows = playlists.iter().enumerate().map(|(i, playlist)| {
        let mut cells = vec![
            Cell::from(format!("{:>idx_w$}", i + 1)),
            Cell::from(truncate_width(&playlist.name, name_w)),
        ];
        if show_creator {
            cells.push(Cell::from(truncate_width(&playlist.creator, creator_w)));
        }
        cells.push(Cell::from(format!(
            "{:>track_w$}",
            human_count(playlist.track_count)
        )));
        Row::new(cells).height(1)
    });

    let mut widths = vec![
        Constraint::Length(idx_w as u16),
        Constraint::Length(name_w as u16),
    ];
    if show_creator {
        widths.push(Constraint::Length(creator_w as u16));
    }
    widths.push(Constraint::Length(track_w as u16));

    Table::new(rows, widths)
        .column_spacing(spacing as u16)
        .highlight_symbol("  ")
        .highlight_spacing(HighlightSpacing::Always)
        .row_highlight_style(
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )
}

#[allow(clippy::too_many_arguments)]
pub fn render_playlists(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    playlists: &[Playlist],
    state: &mut TableState,
    title: &str,
    loading: bool,
    error: Option<&str>,
    empty: &str,
) {
    let title = if error.is_some() && !playlists.is_empty() {
        format!("{title} · 加载失败")
    } else {
        title.to_string()
    };
    let title = truncate_width(&title, area.width.saturating_sub(2) as usize);
    if area.height < 2 || area.width == 0 {
        return;
    }
    let (heading_area, inner) = list_split(area);
    frame.render_widget(Paragraph::new(heading_line(&title, theme)), heading_area);

    if let Some(err) = error
        && playlists.is_empty()
    {
        draw_error(frame, inner, err, theme);
        return;
    }
    if playlists.is_empty() {
        draw_message(
            frame,
            inner,
            empty_state_message(loading, empty),
            Style::default(),
        );
        return;
    }
    let table = playlist_table(playlists, theme, inner.width);
    frame.render_stateful_widget(table, inner, state);
}

/// 输入框显示窗口：光标超过宽度时向左滚动，最左侧用 `…` 表示截断。
fn input_window(input: &str, cursor: usize, available: usize) -> (String, usize) {
    if available == 0 {
        return (String::new(), 0);
    }
    let cursor = cursor.min(input.len());
    let prefix_width = UnicodeWidthStr::width(&input[..cursor]);
    if prefix_width <= available {
        let mut out = String::new();
        let mut width = 0;
        for ch in input.chars() {
            let w = ch.width().unwrap_or(0);
            if width + w > available {
                break;
            }
            width += w;
            out.push(ch);
        }
        return (out, prefix_width.min(available));
    }

    let budget = available.saturating_sub(1).max(1);
    let mut start = cursor;
    let mut used = 0usize;
    for (idx, ch) in input[..cursor].char_indices().rev() {
        let w = ch.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        used += w;
        start = idx;
    }
    let mut out = String::from("…");
    out.push_str(&input[start..cursor]);
    let cursor_col = (1 + used).min(available);
    let mut width = 1 + used;
    for ch in input[cursor..].chars() {
        let w = ch.width().unwrap_or(0);
        if width + w > available {
            break;
        }
        width += w;
        out.push(ch);
    }
    (out, cursor_col)
}

#[allow(clippy::too_many_arguments)]
pub fn render_search(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    input: &str,
    cursor: usize,
    mode: InputMode,
    focus: Focus,
    loading: bool,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    // 单行搜索栏：只占 1 行，出现 / 消失时列表不再整体跳动 3 行。
    let mut label = mode.label().to_string();
    if loading {
        label.push_str(" · 加载中");
    }
    let accent_style = if focus == Focus::Input {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    let prefix = format!(" {label}  › ");
    let prefix_width = UnicodeWidthStr::width(prefix.as_str());
    let (prefix, prefix_width) = if prefix_width + 8 > area.width as usize {
        ("› ".to_string(), 2usize)
    } else {
        (prefix, prefix_width)
    };
    let available = (area.width as usize).saturating_sub(prefix_width);
    let (display, cursor_col) = input_window(input, cursor, available);
    let (shown, style) = if input.is_empty() {
        (
            truncate_width(mode.placeholder(), available),
            Style::default(),
        )
    } else {
        (display, Style::default())
    };

    let line = Line::from(vec![
        Span::styled(prefix, accent_style),
        Span::styled(shown, style),
    ]);
    frame.render_widget(Paragraph::new(line), area);

    if focus == Focus::Input {
        let x = area.x + (prefix_width + cursor_col).min(area.width as usize - 1) as u16;
        frame.set_cursor_position((x, area.y));
    }
}

pub fn render_lyrics(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    lyrics: &Lyrics,
    pos_ms: u64,
    cover: Option<&CoverArt>,
    scroll_offset: &mut i32,
) {
    // 无边框：封面直接浮在内容区，歌词紧接右侧（fox 没有面板边框）。
    let inner = area;
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // 左边封面、右边歌词；面板太矮或太窄时只显示歌词。
    // 封面高度直接跟随面板内部高度缩放，不再绑定固定百分比。
    let show_cover = inner.height >= 5 && inner.width >= 40;
    let lyric_area = if show_cover {
        let art_h = cover_art_height(inner.height);
        let cover_col = (art_h * 2 + COVER_LEFT_PAD + COVER_RIGHT_PAD)
            .max(14)
            .min(inner.width * 2 / 5);
        let chunks = Layout::horizontal([
            Constraint::Length(cover_col),
            Constraint::Length(2),
            Constraint::Min(10),
        ])
        .split(inner);
        render_cover(frame, chunks[0], theme, cover);
        chunks[2]
    } else {
        inner
    };

    if lyric_area.height == 0 || lyric_area.width == 0 {
        return;
    }
    if lyrics.is_empty() {
        draw_message(frame, lyric_area, "暂无歌词", Style::default());
        return;
    }

    // 先按歌词列宽度换行，再记录「当前逻辑行」对应的第一条显示行，
    // 这样自动滚动不会因为长行折行而错位。
    let width = lyric_area.width as usize;
    let current = lyrics.index_at(pos_ms);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_line = 0usize;
    for (i, lyric) in lyrics.lines.iter().enumerate() {
        let is_current = Some(i) == current;
        let style = if is_current {
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            // 非当前行压暗，当前行是唯一焦点（fox 的高亮歌词同理）。
            Style::default()
        };
        for (part_index, part) in wrap_text(&lyric.text, width).into_iter().enumerate() {
            if is_current && part_index == 0 {
                current_line = lines.len();
            }
            lines.push(Line::from(Span::styled(part, style)));
        }
        if let Some(translation) = &lyric.translation {
            for part in wrap_text(translation, width) {
                lines.push(Line::from(Span::styled(
                    part,
                    Style::default().add_modifier(Modifier::ITALIC),
                )));
            }
        }
    }

    let height = lyric_area.height as usize;
    // 有时间轴时自动把当前行放在中间；纯文本从第一行开始。
    let base = if lyrics.timed {
        current_line.saturating_sub(height / 2)
    } else {
        0
    };
    let max_scroll = lines.len().saturating_sub(height);
    // 把偏移限制在当前可滚动范围内，避免连续翻页后积累大量无效偏移。
    let min_offset = -(base as i32);
    let max_offset = max_scroll as i32 - base as i32;
    let offset = (*scroll_offset).clamp(min_offset, max_offset);
    *scroll_offset = offset;
    let scroll = (base as i32 + offset) as usize;
    let paragraph = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .scroll((scroll.min(u16::MAX as usize) as u16, 0));
    frame.render_widget(paragraph, lyric_area);
}

/// 按显示宽度换行，CJK 宽字符不会被从中间截断。
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() || width == 0 {
        return vec![String::new()];
    }
    let mut out = Vec::new();
    let mut line = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used > 0 && used + w > width {
            out.push(std::mem::take(&mut line));
            used = 0;
        }
        line.push(ch);
        used += w;
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// 封面高度跟随歌词面板内部高度变化。
const COVER_SCALE: f64 = 0.62;
/// 封面距面板左边的留白，以及右侧留给歌词列的空隙。
const COVER_LEFT_PAD: u16 = 5;
const COVER_RIGHT_PAD: u16 = 1;

fn cover_art_height(height: u16) -> u16 {
    let art = (height as f64 * COVER_SCALE).round() as u16;
    art.clamp(4, height.saturating_sub(1).max(4))
}

/// 用单色 braille 点阵把封面画成一幅抽象的小图，不再加边框。
fn render_cover(frame: &mut Frame, area: Rect, _theme: &Theme, cover: Option<&CoverArt>) {
    if area.width < 8 || area.height < 5 {
        return;
    }

    let aspect = cover
        .map(|art| art.width as f64 / art.height.max(1) as f64)
        .filter(|aspect| aspect.is_finite() && *aspect > 0.0)
        .unwrap_or(1.0);

    let left_pad = COVER_LEFT_PAD.min(area.width.saturating_sub(1));
    let max_art_w = area.width.saturating_sub(left_pad + COVER_RIGHT_PAD).max(1) as usize;
    let max_art_h = cover_art_height(area.height) as usize;

    // braille 一个字符承载 2x4 个点；终端字符高度约为宽度的两倍，
    // 因此保持视觉比例时字符宽度 = 2 * 字符高度 * 图片宽高比。
    let mut art_h = max_art_h;
    let mut art_w = (art_h as f64 * 2.0 * aspect).round().max(1.0) as usize;
    if art_w > max_art_w {
        art_w = max_art_w;
        art_h = (art_w as f64 / (2.0 * aspect)).round().max(1.0) as usize;
    }
    art_h = art_h.clamp(1, max_art_h);
    art_w = art_w.clamp(1, max_art_w);

    let rect = Rect {
        x: area.x + left_pad,
        y: area.y + (area.height - art_h as u16) / 2,
        width: art_w as u16,
        height: art_h as u16,
    };

    let Some(cover) = cover else {
        draw_message(frame, area, "暂无封面", Style::default());
        return;
    };

    frame.render_widget(
        Paragraph::new(braille_lines(cover, art_w, art_h, Style::default())),
        rect,
    );
}

/// 灰度 → 双线性缩放 → Otsu 阈值 → 2x4 braille。
fn braille_lines(cover: &CoverArt, cols: usize, rows: usize, style: Style) -> Vec<Line<'static>> {
    let src_w = cover.width as usize;
    let src_h = cover.height as usize;
    if src_w == 0 || src_h == 0 || cover.rgb.len() < src_w.saturating_mul(src_h).saturating_mul(3) {
        return Vec::new();
    }

    let gray: Vec<u8> = cover
        .rgb
        .chunks(3)
        .take(src_w * src_h)
        .map(|pixel| {
            (0.299 * pixel[0] as f64 + 0.587 * pixel[1] as f64 + 0.114 * pixel[2] as f64) as u8
        })
        .collect();

    let dst_w = cols * 2;
    let dst_h = rows * 4;
    let scaled = resize_gray(&gray, src_w, src_h, dst_w, dst_h);
    let threshold = otsu_threshold(&scaled);

    let mut lines = Vec::with_capacity(rows);
    for row in 0..rows {
        let mut text = String::with_capacity(cols);
        for col in 0..cols {
            let mut bits = 0u32;
            for dy in 0..4 {
                for dx in 0..2 {
                    let px = col * 2 + dx;
                    let py = row * 4 + dy;
                    if scaled[py * dst_w + px] < threshold {
                        continue;
                    }
                    let dot = match (dx, dy) {
                        (0, 0) => 1,
                        (0, 1) => 2,
                        (0, 2) => 3,
                        (0, 3) => 7,
                        (1, 0) => 4,
                        (1, 1) => 5,
                        (1, 2) => 6,
                        (1, 3) => 8,
                        _ => 0,
                    };
                    bits |= 1 << (dot - 1);
                }
            }
            text.push(char::from_u32(0x2800 + bits).unwrap_or(' '));
        }
        lines.push(Line::from(Span::styled(text, style)));
    }
    lines
}

/// 把灰度图双线性缩放到点阵尺寸。
fn resize_gray(src: &[u8], src_w: usize, src_h: usize, dst_w: usize, dst_h: usize) -> Vec<u8> {
    let mut out = vec![0u8; dst_w * dst_h];
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return out;
    }
    for dy in 0..dst_h {
        let sy =
            ((dy as f64 + 0.5) * src_h as f64 / dst_h as f64 - 0.5).clamp(0.0, (src_h - 1) as f64);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(src_h - 1);
        let fy = sy - y0 as f64;
        for dx in 0..dst_w {
            let sx = ((dx as f64 + 0.5) * src_w as f64 / dst_w as f64 - 0.5)
                .clamp(0.0, (src_w - 1) as f64);
            let x0 = sx.floor() as usize;
            let x1 = (x0 + 1).min(src_w - 1);
            let fx = sx - x0 as f64;
            let top = src[y0 * src_w + x0] as f64 * (1.0 - fx) + src[y0 * src_w + x1] as f64 * fx;
            let bottom =
                src[y1 * src_w + x0] as f64 * (1.0 - fx) + src[y1 * src_w + x1] as f64 * fx;
            out[dy * dst_w + dx] = (top * (1.0 - fy) + bottom * fy).round() as u8;
        }
    }
    out
}

/// Otsu 自动阈值，避免深色 / 浅色封面在固定阈值下糊成一片。
fn otsu_threshold(gray: &[u8]) -> u8 {
    if gray.is_empty() {
        return 128;
    }
    let mut hist = [0u32; 256];
    for &value in gray {
        hist[value as usize] += 1;
    }
    let total = gray.len() as f64;
    let sum: f64 = (0..256).map(|i| i as f64 * hist[i] as f64).sum();

    let mut weight_bg = 0.0;
    let mut sum_bg = 0.0;
    let mut best = -1.0;
    let mut threshold = 128u8;
    for (t, &count) in hist.iter().enumerate() {
        weight_bg += count as f64;
        if weight_bg == 0.0 {
            continue;
        }
        let weight_fg = total - weight_bg;
        if weight_fg == 0.0 {
            break;
        }
        sum_bg += t as f64 * count as f64;
        let mean_bg = sum_bg / weight_bg;
        let mean_fg = (sum - sum_bg) / weight_fg;
        let between = weight_bg * weight_fg * (mean_bg - mean_fg).powi(2);
        if between > best {
            best = between;
            threshold = t as u8;
        }
    }
    threshold
}

#[allow(clippy::too_many_arguments)]
pub fn render_player(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    current: Option<&Song>,
    playing: bool,
    loading: bool,
    pos_ms: u64,
    dur_ms: u64,
    mode: PlayMode,
    status: Option<String>,
) {
    // fox 底栏：无边框。上行整宽进度条，右端 `已播放 ● 总时长 短模式名`；
    // 下行居中 `歌名 歌手`。
    if area.height == 0 || area.width == 0 {
        return;
    }
    let width = area.width as usize;

    // —— 上行：整宽进度条，右端 `cur ● dur 模式`（默认样式）——
    let mode_text = mode.short_label().to_string();
    let note_icon = if loading {
        "…".to_string()
    } else if playing {
        "●".to_string()
    } else if current.is_some() {
        "■".to_string()
    } else {
        "·".to_string()
    };
    let time_text = if dur_ms > 0 {
        format!(
            "{} {} {}",
            format_duration(pos_ms),
            note_icon,
            format_duration(dur_ms)
        )
    } else {
        note_icon
    };
    let trailing = format!("{time_text} {mode_text}");
    let trailing_width = UnicodeWidthStr::width(trailing.as_str());
    let bar_width = width.saturating_sub(trailing_width + 1).max(1);
    let ratio = if dur_ms > 0 {
        (pos_ms.min(dur_ms) as f64 / dur_ms as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled = (bar_width as f64 * ratio).round() as usize;
    let rest = bar_width.saturating_sub(filled);
    let fill_style = if current.is_some() {
        Style::default().fg(theme.accent)
    } else {
        Style::default()
    };
    let bar = vec![
        Span::styled("━".repeat(filled), fill_style),
        Span::styled("┄".repeat(rest), Style::default()),
        Span::raw(" "),
        Span::raw(trailing),
    ];
    frame.render_widget(Paragraph::new(Line::from(bar)), Rect { height: 1, ..area });

    // —— 下行：居中 `歌名 歌手` ——
    if area.height >= 2 {
        let artist = current.map(|song| song.artist_text()).unwrap_or_default();
        let artist_width = UnicodeWidthStr::width(artist.as_str());
        let title_budget = width.saturating_sub(artist_width.min(22) + 1).max(4);
        let title_text = match current {
            Some(song) => truncate_width(&song.name, title_budget),
            None => "未选择歌曲".to_string(),
        };
        let title_style = match current {
            Some(_) => Style::default()
                .fg(theme.bender)
                .add_modifier(Modifier::BOLD),
            None => Style::default(),
        };
        let mut spans = vec![Span::styled(title_text.clone(), title_style)];
        if !artist.is_empty() {
            spans.push(Span::styled(" ", Style::default()));
            spans.push(Span::styled(truncate_width(&artist, 22), Style::default()));
        }

        let total_width: usize = spans.iter().map(|s| s.width()).sum();
        let pad = width.saturating_sub(total_width) / 2;
        // 提示信息（下载/缓存/补全标签/toast/错误）停靠在同一行左侧，
        // 保持歌曲居中；放不下时先截断提示。
        let mut line_spans: Vec<Span<'_>> = Vec::new();
        if let Some(hint) = status.as_deref()
            && pad >= 2
        {
            let clipped = truncate_width(hint, pad.saturating_sub(1));
            let clipped_width = UnicodeWidthStr::width(clipped.as_str());
            line_spans.push(Span::raw(clipped));
            line_spans.push(Span::raw(" ".repeat(pad.saturating_sub(clipped_width))));
        } else {
            line_spans.push(Span::raw(" ".repeat(pad)));
        }
        line_spans.extend(spans);
        frame.render_widget(
            Paragraph::new(Line::from(line_spans)),
            Rect {
                y: area.y + 1,
                height: 1,
                ..area
            },
        );
    }
}

/// 歌名加粗，` — 歌手` 保持普通样式；空间不足时先截断歌名，再截断歌手。
fn progress_text(label: &str, done: u64, total: Option<u64>) -> String {
    match total.filter(|t| *t > 0) {
        Some(total) => format!("{label} {}%", done * 100 / total),
        None => format!("{label}中…"),
    }
}

/// 底部状态行：提示优先，其次加载错误，最后标签补全 / 下载 / 缓存进度。
#[allow(clippy::too_many_arguments)]
/// 底部提示信息（toast / 错误 / 补全标签 / 下载 / 缓存）。
/// 无提示时返回 None；样式统一默认，由播放器第二行左侧停靠。
pub fn status_hint(
    toast: Option<&str>,
    error: Option<&str>,
    tag: Option<(u64, u64, &str)>,
    cache: Option<(u64, Option<u64>)>,
    download: Option<(u64, Option<u64>)>,
) -> Option<String> {
    if let Some(message) = toast {
        Some(message.to_string())
    } else if let Some(error) = error {
        // 把 R 重试放在最前面，窗口再窄也不会先被截掉。
        Some(format!("R 重试 · {error}"))
    } else if let Some((done, total, name)) = tag {
        if total > 0 {
            Some(format!("补全标签 {done}/{total} · {name}"))
        } else {
            Some(format!("补全标签 · {name}"))
        }
    } else if let Some((done, total)) = download {
        Some(progress_text("下载", done, total))
    } else if let Some((done, total)) = cache {
        Some(progress_text("缓存", done, total))
    } else {
        None
    }
}

pub fn render_help(frame: &mut Frame, area: Rect, theme: &Theme, scroll: &mut u16) {
    if area.width < 20 || area.height < 5 {
        return;
    }
    let entries = [
        ("/", "搜索单曲（回车确认，Esc 取消）"),
        ("f", "搜索网易云歌单"),
        ("o", "打开路径 / 链接 / ID / M3U"),
        ("Enter", "打开栏目 / 播放选中歌曲 / 打开歌单"),
        ("Space", "播放 / 暂停"),
        ("n / p", "下一首 / 上一首"),
        ("← / →", "快退 / 快进 5 秒"),
        ("+ / -", "音量"),
        ("r", "切换播放模式"),
        ("l", "显示 / 隐藏歌词（默认打开）"),
        ("v", "歌词纯享：歌词铺满内容区"),
        ("F", "跟随当前歌曲开 / 关"),
        ("PgUp/PgDn", "滚动歌词（也可用 [ / ]）"),
        ("t / T", "补全选中 / 缺失的本地标签并生成副本"),
        ("x / Del", "从最近播放删除"),
        ("d", "下载当前歌曲"),
        ("g / G", "列表首 / 尾"),
        ("Esc", "返回上一级 / 回到首页"),
        ("R", "刷新当前栏目"),
        ("?", "快捷键"),
        ("q", "退出"),
    ];

    let max_width = area.width.saturating_sub(2).max(20);
    let width = ((area.width as u32 * 76 / 100) as u16)
        .clamp(20, max_width)
        .min(area.width);
    let max_height = area.height.saturating_sub(2).max(3);
    let height = ((area.height as u32 * 82 / 100) as u16)
        .min(entries.len() as u16 + 2)
        .min(max_height)
        .max(3);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    // 整行清空：box 边缘可能正好落在双宽字符的第二个单元格上，
    // 只清 box 内部会留下「半个」CJK 字形并吃掉边框单元格。
    // 整行置空则任何字符都被完整擦除，边框一定能正常显示。
    frame.render_widget(
        Clear,
        Rect {
            x: area.x,
            y: rect.y,
            width: area.width,
            height: rect.height,
        },
    );

    // 每项固定一行，超宽描述用省略号截断，滚动位置才和屏幕行一一对应。
    let inner_w = width.saturating_sub(2) as usize;
    let key_w = 12.min(inner_w);
    let desc_w = inner_w.saturating_sub(key_w);
    let lines = entries
        .iter()
        .map(|(key, desc)| {
            let key_text = pad_width(&truncate_width(&format!("{key:>8}  "), key_w), key_w);
            Line::from(vec![
                Span::styled(
                    key_text,
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(truncate_width(desc, desc_w), Style::default()),
            ])
        })
        .collect::<Vec<_>>();

    let inner_h = height.saturating_sub(2) as usize;
    let max_scroll = lines.len().saturating_sub(inner_h);
    let current = (*scroll as usize).min(max_scroll);
    *scroll = current as u16;
    let title = if max_scroll > 0 {
        "快捷键 · ↑/↓ 滚动"
    } else {
        "快捷键"
    };
    let paragraph = Paragraph::new(lines)
        .block(block(title, theme))
        .scroll((current as u16, 0));
    frame.render_widget(paragraph, rect);
}

#[cfg(test)]
mod tests {
    use super::wrap_text;

    #[test]
    fn wraps_cjk_by_display_width() {
        assert_eq!(wrap_text("你好世界", 4), vec!["你好", "世界"]);
        assert_eq!(wrap_text("abc", 2), vec!["ab", "c"]);
    }

    #[test]
    fn does_not_split_wide_char() {
        assert_eq!(wrap_text("a你b", 2), vec!["a", "你", "b"]);
    }

    #[test]
    fn empty_text_keeps_one_line() {
        assert_eq!(wrap_text("", 4), vec![String::new()]);
    }
}
