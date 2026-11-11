use std::path::{Path, PathBuf};

use musicyi_core::{Error, Result};

/// 下载结果：最终文件路径与字节数。
#[derive(Debug, Clone)]
pub struct DownloadOutcome {
    pub path: PathBuf,
    pub bytes: u64,
}

/// 下载到指定路径。若同名文件已存在，会自动加序号，绝不覆盖已有音乐。
pub async fn download_to<F>(
    http: &reqwest::Client,
    url: &str,
    dest: &Path,
    mut on_progress: F,
) -> Result<DownloadOutcome>
where
    F: FnMut(u64, Option<u64>),
{
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| Error::Path(format!("创建目录 {} 失败: {e}", parent.display())))?;
    }
    let dest = unique_path(dest);
    let part = dest.with_file_name(format!(
        "{}.part",
        dest.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download")
    ));
    let _ = tokio::fs::remove_file(&part).await;

    let resp = http
        .get(url)
        .header("Referer", "https://music.163.com/")
        .send()
        .await
        .map_err(|e| Error::Network(format!("下载失败: {e}")))?;
    if !resp.status().is_success() {
        return Err(Error::Network(format!("下载返回 HTTP {}", resp.status())));
    }
    let total = resp.content_length();
    if let Some(total) = total
        && total > 512 * 1024 * 1024
    {
        return Err(Error::Network("音频文件超过 512 MB，已取消".into()));
    }

    let mut file = tokio::fs::File::create(&part)
        .await
        .map_err(|e| Error::Other(format!("创建文件失败: {e}")))?;
    let mut downloaded = 0u64;
    let mut resp = resp;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| Error::Network(format!("读取音频流失败: {e}")))?
    {
        use tokio::io::AsyncWriteExt;
        file.write_all(&chunk)
            .await
            .map_err(|e| Error::Other(format!("写入文件失败: {e}")))?;
        downloaded += chunk.len() as u64;
        on_progress(downloaded, total);
    }
    {
        use tokio::io::AsyncWriteExt;
        file.flush().await.ok();
    }
    drop(file);

    if downloaded == 0 {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(Error::Network("下载内容为空".into()));
    }

    tokio::fs::rename(&part, &dest)
        .await
        .map_err(|e| Error::Other(format!("重命名文件失败: {e}")))?;
    Ok(DownloadOutcome {
        path: dest,
        bytes: downloaded,
    })
}

/// 清理文件名中的非法字符，保留中文与常见标点。
pub fn sanitize(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_space = false;
    for ch in input.chars() {
        let bad = matches!(
            ch,
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'..='\u{1f}'
        );
        if bad {
            out.push('_');
            last_space = false;
        } else if ch.is_whitespace() {
            if !last_space && !out.is_empty() {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    let trimmed = out.trim().trim_matches('.').trim();
    let mut result: String = trimmed.chars().take(120).collect();
    if result.is_empty() {
        result.push_str("未命名");
    }
    result
}

fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("track");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    for i in 1..=999 {
        let name = if ext.is_empty() {
            format!("{stem} ({i})")
        } else {
            format!("{stem} ({i}).{ext}")
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    let ts = musicyi_core::util::now_ts();
    let name = if ext.is_empty() {
        format!("{stem}-{ts}")
    } else {
        format!("{stem}-{ts}.{ext}")
    };
    parent.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn illegal_chars_are_replaced() {
        assert_eq!(sanitize("a/b:c*?"), "a_b_c__");
        assert_eq!(sanitize("  .. "), "未命名");
    }
}
