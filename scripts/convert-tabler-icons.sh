#!/usr/bin/env bash
# 将 Tabler Icons SVG 批量转为小体积 PNG，写入 assets/icons/
#
# 用法:
#   ./scripts/convert-tabler-icons.sh /path/to/tabler-icons-3.44.0
#
# 依赖: rsvg-convert, magick (ImageMagick)

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${1:-}"
DEST="$ROOT_DIR/assets/icons"
SIZE="${ICON_SIZE:-32}"

usage() {
    cat <<'EOF'
将 Tabler Icons outline SVG 转为 PNG

用法:
  ./scripts/convert-tabler-icons.sh <tabler-icons-dir>

环境变量:
  ICON_SIZE   输出边长像素，默认 32
  JOBS        并行任务数，默认 8
EOF
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
fi

[[ -n "$SRC" ]] || die "请指定 tabler-icons 目录"
[[ -d "$SRC/icons/outline" ]] || die "未找到 $SRC/icons/outline"

command -v rsvg-convert >/dev/null 2>&1 || die "未找到 rsvg-convert"
command -v magick >/dev/null 2>&1 || die "未找到 magick (ImageMagick)"

JOBS="${JOBS:-8}"
mkdir -p "$DEST"

convert_one() {
    local svg="$1"
    local base
    base="$(basename "$svg" .svg)"
    local out="$DEST/$base.png"
    rsvg-convert -w "$SIZE" -h "$SIZE" "$svg" \
        | magick - -strip -colors 256 \
            -define png:compression-filter=5 \
            -define png:compression-level=9 \
            "$out"
}

export -f convert_one
export DEST SIZE

log() {
    printf '==> %s\n' "$*" >&2
}

log "转换 outline 图标 (size=${SIZE}px, jobs=${JOBS})"
find "$SRC/icons/outline" -name '*.svg' -print0 \
    | xargs -0 -P "$JOBS" -I {} bash -c 'convert_one "$@"' _ {}

log "写入别名图标"
while IFS='=' read -r alias src_name; do
    [[ -z "$alias" || "$alias" == \#* ]] && continue
    src="$DEST/$src_name.png"
    dst="$DEST/$alias.png"
    if [[ -f "$src" ]]; then
        cp "$src" "$dst"
    else
        printf 'warn: 别名 %s -> %s 源文件不存在\n' "$alias" "$src_name" >&2
    fi
done <<'ALIASES'
paste=clipboard
undo=arrow-back-up
save=device-floppy
new=file-plus
find=search
close=x
ALIASES

count="$(find "$DEST" -maxdepth 1 -name '*.png' | wc -l | tr -d ' ')"
total_size="$(du -sh "$DEST" | awk '{print $1}')"
log "完成: $count 个 PNG, 总大小 $total_size"
