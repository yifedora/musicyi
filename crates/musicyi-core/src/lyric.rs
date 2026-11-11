//! LRC 歌词解析：逐行时间戳 + 翻译。
//!
//! 网易云 `/lyric` 的 `lrc`、`tlyric` 字段可能混有结构化 JSON 行，这里统一成
//! 一条按时间排序的歌词流。

use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct LyricLine {
    pub start_ms: u64,
    pub text: String,
    pub translation: Option<String>,
}

impl LyricLine {
    pub fn is_credit(&self) -> bool {
        const CREDITS: [&str; 12] = [
            "作词",
            "作曲",
            "编曲",
            "制作人",
            "出品",
            "监制",
            "混音",
            "母带",
            "吉他",
            "贝斯",
            "和声",
            "录音",
        ];
        let t = self.text.trim_start();
        CREDITS.iter().any(|p| t.starts_with(p))
            || t.contains("版权")
            || t.starts_with("词:")
            || t.starts_with("曲:")
    }
}

#[derive(Debug, Clone, Default)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
    /// 是否带时间轴。纯文本歌词（雀乐、街声）只能整段展示，无法自动滚动。
    pub timed: bool,
}

impl Lyrics {
    pub fn parse(lrc: &str, tlyric: &str) -> Self {
        let translations = parse_translation(tlyric);
        let mut lines = parse_lrc(lrc);
        for line in &mut lines {
            line.translation = lookup(&translations, line.start_ms);
        }
        if lines.is_empty() {
            return Self {
                lines: parse_plain(lrc),
                timed: false,
            };
        }
        lines.sort_by_key(|l| l.start_ms);
        Self { lines, timed: true }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// 当前时间落在第几行（最后一行 start <= ms）。纯文本没有时间轴。
    pub fn index_at(&self, ms: u64) -> Option<usize> {
        if self.lines.is_empty() || !self.timed {
            return None;
        }
        let idx = self.lines.partition_point(|l| l.start_ms <= ms);
        if idx == 0 { None } else { Some(idx - 1) }
    }
}

/// 无时间轴的纯文本歌词：逐行保留，空行丢弃。
fn parse_plain(raw: &str) -> Vec<LyricLine> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| LyricLine {
            start_ms: 0,
            text: line.to_string(),
            translation: None,
        })
        .collect()
}

fn parse_timestamp(raw: &str) -> Option<u64> {
    // mm:ss.xx / mm:ss.xxx / mm:ss
    let (m, rest) = raw.split_once(':')?;
    let minutes: u64 = m.trim().parse().ok()?;
    let (s, frac) = match rest.split_once('.') {
        Some((s, f)) => (s, Some(f)),
        None => (rest, None),
    };
    let seconds: u64 = s.trim().parse().ok()?;
    let ms = match frac {
        Some(f) => {
            let digits: String = f.trim().chars().take(3).collect();
            let val: u64 = digits.parse().ok()?;
            match digits.len() {
                1 => val * 100,
                2 => val * 10,
                _ => val,
            }
        }
        None => 0,
    };
    Some(minutes * 60_000 + seconds * 1000 + ms)
}

fn parse_lrc(raw: &str) -> Vec<LyricLine> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // 结构化 JSON 行（常见于作词/作曲等元信息）
        if line.starts_with('{') {
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                let t = v.get("t").and_then(Value::as_u64).unwrap_or(0);
                let text = v
                    .get("c")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.get("tx").and_then(Value::as_str))
                            .collect::<String>()
                    })
                    .unwrap_or_default();
                let text = text.trim().to_string();
                if !text.is_empty() {
                    out.push(LyricLine {
                        start_ms: t,
                        text,
                        translation: None,
                    });
                }
            }
            continue;
        }

        let Some(after_open) = line.strip_prefix('[') else {
            continue;
        };
        let Some((tag, body)) = after_open.split_once(']') else {
            continue;
        };
        if let Some(ms) = parse_timestamp(tag.trim()) {
            let text = body.trim().to_string();
            if !text.is_empty() {
                out.push(LyricLine {
                    start_ms: ms,
                    text,
                    translation: None,
                });
            }
        }
    }
    out
}

fn parse_translation(raw: &str) -> Vec<(u64, String)> {
    if raw.trim().is_empty() {
        return Vec::new();
    }
    parse_lrc(raw)
        .into_iter()
        .map(|l| (l.start_ms, l.text))
        .collect()
}

fn lookup(table: &[(u64, String)], ms: u64) -> Option<String> {
    if table.is_empty() {
        return None;
    }
    let idx = table.partition_point(|(t, _)| *t <= ms);
    let candidates = [
        idx.checked_sub(1).and_then(|i| table.get(i)),
        table.get(idx),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|(t, _)| t.abs_diff(ms) <= 300)
        .map(|(_, text)| text.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_basic() {
        let l = Lyrics::parse("[00:01.00]a\n[00:02.50]b\n", "");
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.index_at(2000), Some(0));
        assert_eq!(l.index_at(2600), Some(1));
    }

    #[test]
    fn mixed_new_format() {
        let raw = "{\"t\":0,\"c\":[{\"tx\":\"作词: \"},{\"tx\":\"黄家驹\"}]}\n[00:18.466]今天我 寒夜里看雪飘过";
        let l = Lyrics::parse(raw, "");
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.lines[1].text, "今天我 寒夜里看雪飘过");
        assert!(l.lines[0].is_credit());
    }

    #[test]
    fn translation_is_attached() {
        let l = Lyrics::parse("[00:01.00]hello\n[00:03.00]world\n", "[00:01.00]你好\n");
        assert_eq!(l.lines[0].translation.as_deref(), Some("你好"));
        assert!(l.lines[1].translation.is_none());
    }

    #[test]
    fn plain_text_fallback() {
        let l = Lyrics::parse("第一行\n\n第二行\n第三行\n", "");
        assert!(!l.timed);
        assert_eq!(l.lines.len(), 3);
        assert_eq!(l.lines[1].text, "第二行");
        assert_eq!(l.index_at(10_000), None);
    }
}
