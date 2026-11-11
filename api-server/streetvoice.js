'use strict';

// 街声（streetvoice.cn）目录与播放：
//   GET  /musicyi/catalog/streetvoice/realtime   即时榜（前端已不使用）
//   GET  /musicyi/catalog/streetvoice/recommend  街声推荐 30 首
//   GET  /musicyi/catalog/streetvoice/:id        单曲详情（补时长/专辑/歌词）
//   GET  /musicyi/stream/streetvoice/:id         HLS -> MP3，供 Rust 端缓存播放
//
// HLS 分片实际是 MP3-in-TS，用 ffmpeg 无损转封装即可，不需要重新编码。

const { spawn } = require('child_process');
const fsp = require('fs/promises');
const os = require('os');
const path = require('path');

const UA =
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 ' +
  '(KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36';

const BASE = 'https://www.streetvoice.cn';
const REALTIME_URL = `${BASE}/music/charts/realtime/all/`;
/// 推荐页每页 20 首，路径第一段是页码（0 起）。
const recommendPageUrl = (page) => `${BASE}/music/browse/${page}/recommend/latest/`;
const RECOMMEND_URL = recommendPageUrl(0);
const LIST_TTL_MS = 10 * 60 * 1000;
const DETAIL_TTL_MS = 30 * 60 * 1000;
const LIST_LIMIT = 30;
const STREAM_DIR = path.join(os.tmpdir(), 'musicyi-streetvoice');

const LIST_SOURCES = {
  realtime: { url: REALTIME_URL, label: '即时榜' },
  recommend: { url: RECOMMEND_URL, pageUrl: recommendPageUrl, label: '街声推荐' },
};

const listCaches = new Map();
const detailCache = new Map();
const streamJobs = new Map();

async function fetchText(url, init) {
  const resp = await fetch(url, {
    redirect: 'follow',
    signal: AbortSignal.timeout(20000),
    ...init,
    headers: {
      'User-Agent': UA,
      Accept: 'text/html,application/json',
      'Accept-Language': 'zh-CN,zh;q=0.9,en;q=0.8',
      ...(init && init.headers ? init.headers : {}),
    },
  });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.text();
}

function decodeEntities(text) {
  return String(text || '')
    .replace(/&amp;/g, '&')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&#x27;/g, "'")
    .replace(/&#(\d+);/g, (_, n) => String.fromCodePoint(Number(n)))
    .replace(/&#x([0-9a-f]+);/gi, (_, n) => String.fromCodePoint(parseInt(n, 16)));
}

function stripTags(text) {
  return decodeEntities(String(text || '').replace(/<[^>]*>/g, '')).replace(/\s+/g, ' ').trim();
}

/** 从 `<h4 class="text-truncate">` 里取 `<a>` 的文本，兼容换行与空白。 */
function headingText(chunk, tag) {
  const block = chunk.match(
    new RegExp(`<${tag}\\b[^>]*class="[^"]*text-truncate[^"]*"[^>]*>([\\s\\S]*?)</${tag}>`, 'i')
  );
  if (!block) return '';
  const anchor = block[1].match(/<a\b[^>]*>([\s\S]*?)<\/a>/i);
  return stripTags(anchor ? anchor[1] : block[1]);
}

function parseList(html, limit = LIST_LIMIT) {
  const chunks = html.split(/<li class="list-group-item[^"]*work-item[^"]*"/).slice(1);
  const songs = [];
  const seen = new Set();

  for (const chunk of chunks) {
    const id = chunk.match(/data-id="(\d+)"/)?.[1];
    if (!id || seen.has(id)) continue;
    const cover = chunk.match(/<img\s+src="([^"]+)"/i)?.[1] || null;
    const name = headingText(chunk, 'h4');
    if (!name) continue;
    const artist = headingText(chunk, 'h5');
    seen.add(id);
    songs.push({
      id,
      name,
      artists: artist ? [artist] : [],
      album: '',
      duration_ms: 0,
      cover_url: cover ? decodeEntities(cover) : null,
    });
    if (songs.length >= limit) break;
  }
  return songs;
}

async function buildList(kind) {
  const source = LIST_SOURCES[kind];
  if (!source) throw new Error(`未知街声列表：${kind}`);
  // 站点改用 htmx：不带 HX-Request 时只返回外壳，列表在 pjax 片段里。
  // 推荐页按页码分页，每页 20 首；跨页可能有重复，一边翻页一边去重，
  // 直到凑满 LIST_LIMIT 或页面不再返回作品。
  const maxPages = source.pageUrl ? 10 : 1;
  const songs = [];
  const seen = new Set();
  for (let page = 0; page < maxPages && songs.length < LIST_LIMIT; page++) {
    const url = source.pageUrl ? source.pageUrl(page) : source.url;
    const html = await fetchText(url, {
      headers: { 'HX-Request': 'true' },
    });
    const pageSongs = parseList(html, LIST_LIMIT);
    if (!pageSongs.length) break;
    for (const song of pageSongs) {
      if (seen.has(song.id)) continue;
      seen.add(song.id);
      songs.push(song);
      if (songs.length >= LIST_LIMIT) break;
    }
  }
  if (!songs.length) throw new Error('列表页面里没有找到歌曲');
  return songs.slice(0, LIST_LIMIT);
}

function listHandler(kind) {
  return async function handleList(_req, res) {
    const now = Date.now();
    const cached = listCaches.get(kind);
    if (cached && now - cached.at < LIST_TTL_MS) {
      res.json({ ok: true, songs: cached.songs });
      return;
    }
    try {
      const songs = await buildList(kind);
      listCaches.set(kind, { at: now, songs });
      console.log(`[streetvoice] ${LIST_SOURCES[kind].label}刷新：${songs.length} 首`);
      res.json({ ok: true, songs });
    } catch (error) {
      console.log(`[streetvoice] ${LIST_SOURCES[kind].label}失败: ${error.message}`);
      res.status(502).json({ ok: false, error: error.message });
    }
  };
}

const handleRealtime = listHandler('realtime');
const handleRecommend = listHandler('recommend');

async function loadDetail(id) {
  const now = Date.now();
  const cached = detailCache.get(id);
  if (cached && now - cached.at < DETAIL_TTL_MS) return cached.detail;

  const text = await fetchText(`${BASE}/api/v5/song/${id}/`);
  const json = JSON.parse(text);
  const detail = {
    id: String(json.id || id),
    name: String(json.name || ''),
    duration_ms: Math.round(Number(json.length || 0) * 1000),
    album: json.album && json.album.name ? String(json.album.name) : '',
    cover_url: json.image ? String(json.image) : null,
    lyrics: typeof json.lyrics === 'string' ? json.lyrics : '',
    lyrics_is_lrc: Boolean(json.lyrics_is_lrc),
  };
  detailCache.set(id, { at: now, detail });
  return detail;
}

async function handleDetail(req, res) {
  const id = String(req.params.id || '').trim();
  if (!/^\d+$/.test(id)) {
    res.status(400).json({ ok: false, error: '无效的街声 ID' });
    return;
  }
  try {
    const detail = await loadDetail(id);
    res.json({
      ok: true,
      id: detail.id,
      name: detail.name,
      duration_ms: detail.duration_ms,
      album: detail.album,
      cover_url: detail.cover_url,
      lyrics: detail.lyrics,
      lyrics_is_lrc: detail.lyrics_is_lrc,
    });
  } catch (error) {
    res.status(502).json({ ok: false, error: error.message });
  }
}

async function hlsUrl(id) {
  const text = await fetchText(`${BASE}/api/v5/song/${id}/hls/file/`, {
    method: 'POST',
    headers: {
      Referer: REALTIME_URL,
      'X-Requested-With': 'XMLHttpRequest',
    },
  });
  const json = JSON.parse(text);
  if (!json.file) throw new Error('没有返回 HLS 地址');
  return String(json.file);
}

function runFfmpeg(input, output, copyAudio) {
  return new Promise((resolve, reject) => {
    const args = [
      '-y',
      '-hide_banner',
      '-loglevel',
      'error',
      '-i',
      input,
      '-map',
      '0:a:0',
      '-vn',
      ...(copyAudio
        ? ['-c:a', 'copy']
        : ['-c:a', 'libmp3lame', '-b:a', '128k']),
      '-f',
      'mp3',
      output,
    ];
    const child = spawn('ffmpeg', args, { stdio: ['ignore', 'ignore', 'pipe'] });
    let stderr = '';
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      reject(new Error('ffmpeg 超时'));
    }, 180000);
    child.stderr.on('data', (data) => {
      if (stderr.length < 4000) stderr += data.toString();
    });
    child.on('error', (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.on('close', (code) => {
      clearTimeout(timer);
      if (code === 0) resolve();
      else reject(new Error(stderr.trim() || `ffmpeg 退出码 ${code}`));
    });
  });
}

async function convertToMp3(id) {
  const final = path.join(STREAM_DIR, `${id}.mp3`);
  const stat = await fsp.stat(final).catch(() => null);
  if (stat && stat.size > 100 * 1024) return final;

  const hls = await hlsUrl(id);
  await fsp.mkdir(STREAM_DIR, { recursive: true });
  const temp = path.join(STREAM_DIR, `${id}.${process.pid}.part.mp3`);

  try {
    await runFfmpeg(hls, temp, true).catch(async (error) => {
      console.log(`[streetvoice] ${id} 直接转封装失败，改为转码: ${error.message}`);
      await fsp.rm(temp, { force: true });
      await runFfmpeg(hls, temp, false);
    });
    const out = await fsp.stat(temp);
    if (out.size < 100 * 1024) throw new Error('转出的音频过小');
    await fsp.rename(temp, final);
    console.log(`[streetvoice] ${id} 已转好 ${Math.round(out.size / 1024)} KiB`);
    return final;
  } catch (error) {
    await fsp.rm(temp, { force: true });
    throw error;
  }
}

async function handleStream(req, res) {
  const id = String(req.params.id || '').trim();
  if (!/^\d+$/.test(id)) {
    res.status(400).json({ ok: false, error: '无效的街声 ID' });
    return;
  }
  try {
    let job = streamJobs.get(id);
    if (!job) {
      job = convertToMp3(id).finally(() => streamJobs.delete(id));
      streamJobs.set(id, job);
    }
    const file = await job;
    res.setHeader('Cache-Control', 'no-store');
    res.type('audio/mpeg');
    res.sendFile(file, (error) => {
      if (error && !res.headersSent) {
        res.status(500).json({ ok: false, error: error.message });
      }
    });
  } catch (error) {
    console.log(`[streetvoice] 流失败 ${id}: ${error.message}`);
    if (!res.headersSent) res.status(502).json({ ok: false, error: error.message });
  }
}

module.exports = { handleRealtime, handleRecommend, handleDetail, handleStream };
