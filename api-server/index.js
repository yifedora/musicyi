#!/usr/bin/env node
'use strict';

// 启动网易云 API，并在同一个 Express 实例上挂载 musicyi 的雀乐 / 街声目录路由。
// 这样不需要额外端口或额外进程，默认仍是 3000。

const fs = require('fs');
const os = require('os');
const path = require('path');

async function start() {
  const tokenFile = path.resolve(os.tmpdir(), 'anonymous_token');
  if (!fs.existsSync(tokenFile)) {
    fs.writeFileSync(tokenFile, '', 'utf-8');
  }

  const generateConfig = require('netease-cloud-music-api-alger/generateConfig');
  await generateConfig();

  const { serveNcmApi } = require('netease-cloud-music-api-alger/server');
  // 默认只监听本机，避免把音乐 API 暴露到局域网。需要给其他设备用时
  // 可显式设置 HOST=0.0.0.0。
  const host = process.env.HOST || '127.0.0.1';
  const app = await serveNcmApi({ checkVersion: true, host });

  // serveNcmApi 内部已经 listen，但 Express 仍允许继续挂载路由。
  // 路由自身会收到前面注册的 apicache 中间件影响；调用方需要带
  // x-apicache-bypass 头，避免不同歌曲共用同一个缓存键。
  app.get('/musicyi/health', (_req, res) => {
    res.json({ ok: true, catalog: true, stream: true });
  });
  app.get('/musicyi/catalog/indie', require('./indie'));
  const streetvoice = require('./streetvoice');
  // 具体列表路由必须放在 :id 之前，否则 realtime / recommend 会被当成 ID。
  app.get('/musicyi/catalog/streetvoice/realtime', streetvoice.handleRealtime);
  app.get('/musicyi/catalog/streetvoice/recommend', streetvoice.handleRecommend);
  app.get('/musicyi/catalog/streetvoice/:id', streetvoice.handleDetail);
  app.get('/musicyi/stream/streetvoice/:id', streetvoice.handleStream);
}

start().catch((error) => {
  console.error('API 启动失败:', error);
  process.exit(1);
});
