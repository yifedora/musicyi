#!/usr/bin/env bash
# 停止本机的 netease-cloud-music-api 服务。
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PID_FILE="$DIR/server.pid"
PORT="${PORT:-3000}"

port_pid() {
    ss -ltnp "sport = :${PORT}" 2>/dev/null |
        sed -n 's/.*pid=\([0-9]\+\).*/\1/p' |
        head -1
}

pid=""
if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
    pid="$(cat "$PID_FILE")"
else
    pid="$(port_pid)"
fi

if [[ -z "$pid" ]]; then
    echo "没有找到运行中的 API 服务（端口 ${PORT}）"
    rm -f "$PID_FILE"
    exit 0
fi

kill "$pid" 2>/dev/null || true
for _ in $(seq 1 20); do
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.2
done
rm -f "$PID_FILE"

if kill -0 "$pid" 2>/dev/null; then
    echo "API 未能正常退出（pid $pid）" >&2
    exit 1
fi
echo "API 已停止（pid $pid）"
