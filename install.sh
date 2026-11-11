#!/usr/bin/env bash
# musicyi 安装脚本：构建 release 并安装到 ~/.local/bin（可用 PREFIX 覆盖）。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PREFIX="${PREFIX:-$HOME/.local/bin}"
BIN="$ROOT/target/release/musicyi"

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m警告:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31m错误:\033[0m %s\n' "$*" >&2; exit 1; }

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    cat <<EOF
用法: ./install.sh [--uninstall]

环境变量:
  PREFIX   安装前缀目录，默认 \$HOME/.local/bin

说明:
  - 从源码构建 release 版本并安装 musicyi 可执行文件
  - 不会覆盖或删除 ~/.config/musicyi 与 ~/.local/share/musicyi 中的数据
  - --uninstall 只删除已安装的可执行文件，配置与数据需手动清理
EOF
    exit 0
fi

if [[ "${1:-}" == "--uninstall" ]]; then
    if [[ -e "$PREFIX/musicyi" ]]; then
        rm -f "$PREFIX/musicyi"
        info "已删除 $PREFIX/musicyi"
        info "配置与数据仍保留：~/.config/musicyi、~/.local/share/musicyi"
    else
        info "$PREFIX/musicyi 不存在，无需卸载"
    fi
    exit 0
fi

command -v cargo >/dev/null 2>&1 || die "未找到 cargo，请先安装 Rust 1.88+（https://rustup.rs）"
command -v pkg-config >/dev/null 2>&1 || warn "未找到 pkg-config，若编译 cpal 失败请安装它"

if [[ "$(uname -s)" == "Linux" ]]; then
    if command -v pkg-config >/dev/null 2>&1 && ! pkg-config --exists alsa; then
        die "缺少 ALSA 开发库。Fedora: sudo dnf install alsa-lib-devel；Debian/Ubuntu: sudo apt install libasound2-dev"
    fi
fi

info "构建 release 版本…"
cd "$ROOT"
cargo build --release

[[ -x "$BIN" ]] || die "构建完成但未找到 $BIN"

info "安装到 $PREFIX/musicyi"
mkdir -p "$PREFIX"
install -Dm755 "$BIN" "$PREFIX/musicyi"

info "完成。运行：musicyi"
case ":$PATH:" in
    *":$PREFIX:"*) ;;
    *) warn "$PREFIX 不在 PATH 中，请把它加入 PATH 后重新打开终端" ;;
esac

if command -v node >/dev/null 2>&1; then
    info "首次使用前请启动 API：$ROOT/api-server/start.sh"
else
    warn "未找到 node，无法运行本机网易云 API；请安装 Node.js 18+ 后执行 api-server/start.sh"
fi
