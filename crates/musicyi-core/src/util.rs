use std::path::PathBuf;

use unicode_width::UnicodeWidthStr;

/// 毫秒 → `mm:ss`（超过 1 小时则为 `h:mm:ss`）。
pub fn format_duration(ms: u64) -> String {
    let total = ms / 1000;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// 按终端显示宽度截断（CJK 计 2 列），超长补 `…`。
pub fn truncate_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let width = UnicodeWidthStr::width(text);
    if width <= max_width {
        return text.to_string();
    }
    let ellipsis = "…";
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w + 1 > max_width {
            break;
        }
        used += w;
        out.push(ch);
    }
    out.push_str(ellipsis);
    out
}

/// 按显示宽度右侧补齐空格。
pub fn pad_width(text: &str, width: usize) -> String {
    let w = UnicodeWidthStr::width(text);
    let mut s = text.to_string();
    if w < width {
        s.push_str(&" ".repeat(width - w));
    }
    s
}

/// 大数字 → `12.3万` / `1.2亿`。
pub fn human_count(n: u64) -> String {
    if n >= 100_000_000 {
        format!("{:.1}亿", n as f64 / 100_000_000.0)
    } else if n >= 10_000 {
        format!("{:.1}万", n as f64 / 10_000.0)
    } else {
        n.to_string()
    }
}

pub fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

/// 展开 `~` / `~/...`，其余原样返回。
pub fn expand_home(input: &str) -> PathBuf {
    let trimmed = input.trim();
    if trimmed == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    }
    if let Some(rest) = trimmed.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(trimmed)
}
