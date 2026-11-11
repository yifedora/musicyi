use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use musicyi_core::{Error, Result};

/// 音频下载缓存（LRU，按 mtime 淘汰）。
#[derive(Debug, Clone)]
pub struct AudioCache {
    dir: PathBuf,
    limit_bytes: u64,
}

impl AudioCache {
    pub fn new(dir: PathBuf, limit_mb: u64) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self {
            dir,
            limit_bytes: limit_mb.saturating_mul(1024 * 1024).max(64 * 1024 * 1024),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path_for(&self, key: &str, ext: &str) -> PathBuf {
        let ext = sanitize_ext(ext);
        self.dir.join(format!("{:016x}.{ext}", fnv1a(key)))
    }

    pub fn existing(&self, key: &str, ext: &str) -> Option<PathBuf> {
        let p = self.path_for(key, ext);
        match std::fs::metadata(&p) {
            Ok(m) if m.len() > 0 => Some(p),
            _ => None,
        }
    }

    /// 下载到缓存。`on_progress(已下载字节, 总字节)`。
    pub async fn fetch<F>(
        &self,
        http: &reqwest::Client,
        url: &str,
        key: &str,
        ext: &str,
        mut on_progress: F,
    ) -> Result<PathBuf>
    where
        F: FnMut(u64, Option<u64>),
    {
        let final_path = self.path_for(key, ext);
        if let Some(p) = self.existing(key, ext) {
            // 触碰 mtime，延长保留时间
            if let Ok(f) = std::fs::File::open(&p) {
                let _ = f.set_modified(SystemTime::now());
            }
            return Ok(p);
        }

        let part = final_path.with_extension(format!("{}.part", sanitize_ext(ext)));
        let _ = std::fs::remove_file(&part);

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

        let mut file = tokio::fs::File::create(&part)
            .await
            .map_err(|e| Error::Other(format!("创建缓存文件失败: {e}")))?;
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
                .map_err(|e| Error::Other(format!("写入缓存失败: {e}")))?;
            downloaded += chunk.len() as u64;
            on_progress(downloaded, total);
        }
        {
            use tokio::io::AsyncWriteExt;
            file.flush().await.ok();
        }
        drop(file);

        if downloaded == 0 {
            let _ = std::fs::remove_file(&part);
            return Err(Error::Network("下载内容为空".into()));
        }

        tokio::fs::rename(&part, &final_path)
            .await
            .map_err(|e| Error::Other(format!("重命名缓存失败: {e}")))?;
        self.enforce_limit();
        Ok(final_path)
    }

    pub fn enforce_limit(&self) {
        let mut entries = collect_files(&self.dir);
        let total: u64 = entries.iter().map(|e| e.1).sum();
        if total <= self.limit_bytes {
            return;
        }
        // 旧的先删
        entries.sort_by_key(|e| e.0);
        let mut current = total;
        for (mtime, size, path) in entries {
            if current <= self.limit_bytes * 9 / 10 {
                break;
            }
            if path.extension().is_some_and(|e| e == "part") {
                continue;
            }
            let _ = std::fs::remove_file(&path);
            current = current.saturating_sub(size);
            tracing::debug!("缓存淘汰: {}", path.display());
            let _ = mtime;
        }
    }

    pub fn total_size(&self) -> u64 {
        collect_files(&self.dir).iter().map(|e| e.1).sum()
    }

    /// 清理超过一天仍残留的半成品。
    pub fn clean_partials(&self) {
        let cutoff = SystemTime::now() - Duration::from_secs(24 * 3600);
        if let Ok(read) = std::fs::read_dir(&self.dir) {
            for entry in read.flatten() {
                let p = entry.path();
                if p.extension().is_some_and(|e| e == "part")
                    && let Ok(meta) = entry.metadata()
                    && meta.modified().is_ok_and(|m| m < cutoff)
                {
                    let _ = std::fs::remove_file(p);
                }
            }
        }
    }
}

fn collect_files(dir: &Path) -> Vec<(SystemTime, u64, PathBuf)> {
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        out.push((
            meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            meta.len(),
            path,
        ));
    }
    out
}

fn sanitize_ext(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "flac" => "flac",
        "m4a" | "mp4" | "aac" => "m4a",
        "ogg" | "opus" => "ogg",
        "wav" => "wav",
        _ => "mp3",
    }
}

/// FNV-1a 64，稳定且无需依赖。
fn fnv1a(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in input.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}
