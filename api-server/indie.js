'use strict';

// 雀乐（indie.cn）歌单目录：首页取最新 vol 编号，再逐个抓取 vol 页里的
// `songList`（Next.js flight 数据），一次性把最新 10 期歌单和歌曲返回给客户端。
//
// 路由：GET /musicyi/catalog/indie
// 响应：{ playlists: [{ id, name, cover_url, track_count, songs: [...] }] }

const UA =
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 ' +
  '(KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36';

const HOME_URL = 'https://www.indie.cn/';
const VOL_URL = (id) => `https://www.indie.cn/vol/${id}`;
const CACHE_TTL_MS = 15 * 60 * 1000;
const PLAYLIST_COUNT = 10;

let cache = null;

async function fetchText(url) {
  const resp = await fetch(url, {
    redirect: 'follow',
    signal: AbortSignal.timeout(20000),
    headers: {
      'User-Agent': UA,
      Accept: 'text/html,application/xhtml+xml',
      'Accept-Language': 'zh-CN,zh;q=0.9,en;q=0.8',
    },
  });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.text();
}

/** 把 Next.js 的 flight 流拼成一段可搜索的字符串。 */
function parseFlight(html) {
  const re = /self\.__next_f\.push\(\[1,("(?:[^"\\]|\\.)*")\]\)/g;
  let out = '';
  for (const match of html.matchAll(re)) {
    try {
      out += JSON.parse(match[1]);
    } catch (_) {
      // 跳过无法解析的片段。
    }
  }
  return out;
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
  return decodeEntities(String(text || '').replace(/<[^>]*>/g, '')).trim();
}

/** 从 marker 之后截取一个完整的 JSON 数组（正确处理字符串转义）。 */
function extractArrayAfter(text, marker) {
  const at = text.indexOf(marker);
  if (at < 0) return null;
  const start = text.indexOf('[', at + marker.length);
  if (start < 0) return null;
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let i = start; i < text.length; i += 1) {
    const ch = text[i];
    if (inString) {
      if (escaped) escaped = false;
      else if (ch === '\\') escaped = true;
      else if (ch === '"') inString = false;
      continue;
    }
    if (ch === '"') inString = true;
    else if (ch === '[') depth += 1;
    else if (ch === ']') {
      depth -= 1;
      if (depth === 0) return text.slice(start, i + 1);
    }
  }
  return null;
}

function parseDuration(raw) {
  if (typeof raw === 'number') return raw > 1000 ? raw : raw * 1000;
  const parts = String(raw || '').trim().split(':');
  if (parts.length !== 2) return 0;
  const minutes = Number(parts[0]);
  const seconds = Number(parts[1]);
  if (!Number.isFinite(minutes) || !Number.isFinite(seconds)) return 0;
  return Math.round((minutes * 60 + seconds) * 1000);
}

function songFromRaw(raw) {
  if (!raw || !raw.title) return null;
  return {
    id: String(raw.id || `${raw.journalNo}_${raw.songNo}`),
    name: String(raw.title),
    artists: raw.artist ? [String(raw.artist)] : [],
    album: raw.album ? String(raw.album) : '',
    duration_ms: parseDuration(raw.duration),
    cover_url: raw.pic || null,
    play_url: raw.src || null,
    lyric_url: raw.lrc || null,
  };
}

function volumeIds(flight) {
  const ids = new Set();
  for (const match of flight.matchAll(/"href":"\/vol\/(\d+)"/g)) {
    ids.add(match[1]);
  }
  return [...ids]
    .map(Number)
    .filter((n) => Number.isFinite(n) && n > 0)
    .sort((a, b) => b - a)
    .slice(0, PLAYLIST_COUNT)
    .map(String);
}

async function loadVolume(id) {
  const html = await fetchText(VOL_URL(id));
  const flight = parseFlight(html);
  const titleMatch = html.match(/<title>([^<]*)<\/title>/i);
  let name = titleMatch ? stripTags(titleMatch[1]) : `Vol ${id}`;
  name = name.replace(/\s*[-–—]\s*雀乐\s*$/, '').trim() || `Vol ${id}`;

  const arrayText = extractArrayAfter(flight, '"songList":');
  if (!arrayText) throw new Error('页面里没有 songList');
  const rawSongs = JSON.parse(arrayText);
  const songs = rawSongs.map(songFromRaw).filter(Boolean);
  if (!songs.length) throw new Error('歌单为空');

  return {
    id: String(id),
    name,
    cover_url: songs[0].cover_url || null,
    track_count: songs.length,
    songs,
  };
}

async function buildCatalog() {
  const home = await fetchText(HOME_URL);
  const ids = volumeIds(parseFlight(home));
  if (!ids.length) throw new Error('首页没有找到歌单');

  const results = await Promise.all(
    ids.map(async (id) => {
      try {
        return await loadVolume(id);
      } catch (error) {
        console.log(`[indie] vol ${id} 失败: ${error.message}`);
        return null;
      }
    })
  );
  const playlists = results.filter(Boolean);
  if (!playlists.length) throw new Error('所有歌单都抓取失败');
  return playlists;
}

async function handleCatalog(_req, res) {
  const now = Date.now();
  if (cache && now - cache.at < CACHE_TTL_MS) {
    res.json({ ok: true, playlists: cache.playlists });
    return;
  }
  try {
    const playlists = await buildCatalog();
    cache = { at: now, playlists };
    console.log(`[indie] 目录刷新：${playlists.length} 个歌单`);
    res.json({ ok: true, playlists });
  } catch (error) {
    console.log(`[indie] 目录失败: ${error.message}`);
    res.status(502).json({ ok: false, error: error.message });
  }
}

module.exports = handleCatalog;
