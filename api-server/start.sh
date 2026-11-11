#!/usr/bin/env bash
# 启动本机的 netease-cloud-music-api 服务（默认 3000 端口）。
# 已在运行时直接返回，可重复执行。
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PORT="${PORT:-3000}"
# 默认只监听本机；确需局域网访问时显式设置 HOST=0.0.0.0。
HOST="${HOST:-127.0.0.1}"
HEALTH_URL="http://127.0.0.1:${PORT}/search/hot/detail"

port_pid() {
    ss -ltnp "sport = :${PORT}" 2>/dev/null |
        sed -n 's/.*pid=\([0-9]\+\).*/\1/p' |
        head -1
}

if curl -fsS -m 2 "$HEALTH_URL" >/dev/null 2>&1; then
    # 已在运行（可能是手工启动的），尽量记下 PID，方便 stop.sh 停止。
    pid="$(port_pid)"
    if [[ -n "$pid" ]]; then
        echo "$pid" >"$DIR/server.pid"
    fi
    if ! curl -fsS -m 2 "http://127.0.0.1:${PORT}/musicyi/health" >/dev/null 2>&1; then
        echo "警告：运行中的 API 是旧版本（没有雀乐 / 街声目录路由），请先执行 ./stop.sh 再重新启动。" >&2
    fi
    echo "API 已在运行：http://127.0.0.1:${PORT}${pid:+（pid $pid）}"
    exit 0
fi

cd "$DIR"
nohup env PORT="$PORT" HOST="$HOST" node index.js >>server.log 2>&1 &
echo $! >server.pid

for _ in $(seq 1 40); do
    if curl -fsS -m 1 "$HEALTH_URL" >/dev/null 2>&1 &&
        curl -fsS -m 1 "http://127.0.0.1:${PORT}/musicyi/health" >/dev/null 2>&1; then
        echo "API 启动成功：http://127.0.0.1:${PORT}（pid $(cat server.pid)）"
        exit 0
    fi
    sleep 0.5
done

echo "API 启动失败，请查看：$DIR/server.log" >&2
exit 1
