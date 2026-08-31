#!/usr/bin/env bash
# 根据折叠桌宠图标 assets/icons/pet-cat.png 生成 macOS 应用图标
#
# 产出：
#   assets/AppIcon-1024.png   # 1024 主图
#   assets/AppIcon.icns       # 打包进 .app 的图标
#
# 用法：
#   ./scripts/generate-app-icon.sh

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

log() {
    printf '==> %s\n' "$*" >&2
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

[[ "$(uname -s)" == "Darwin" ]] || die "此脚本仅支持 macOS"
command -v python3 >/dev/null 2>&1 || die "未找到 python3"
command -v sips >/dev/null 2>&1 || die "未找到 sips"
command -v iconutil >/dev/null 2>&1 || die "未找到 iconutil"
[[ -f "$ROOT_DIR/assets/icons/pet-cat.png" ]] || die "未找到 assets/icons/pet-cat.png"

log "准备临时 Python 环境（Pillow）"
VENV="$(mktemp -d)/venv"
python3 -m venv "$VENV"
"$VENV/bin/pip" install -q pillow

log "生成应用图标"
"$VENV/bin/python" "$ROOT_DIR/scripts/generate_app_icon.py"
rm -rf "$(dirname "$VENV")"
