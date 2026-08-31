#!/usr/bin/env bash
# 从系统 Hiragino Sans GB 生成 assets/fonts/ui-cjk.ttf 子集字体
# 依赖：python3 + fonttools（可用临时 venv）
#
# 用法：
#   ./scripts/rebuild-ui-font.sh
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHARSET="$ROOT_DIR/scripts/cjk_charset_lean.txt"
# 若需更全字形覆盖（窗口标题等），改用 scripts/cjk_charset.txt 再重建
if [[ ! -f "$CHARSET" ]]; then
  CHARSET="$ROOT_DIR/scripts/cjk_charset.txt"
fi
OUT="$ROOT_DIR/assets/fonts/ui-cjk.ttf"
SRC_TTC="/System/Library/Fonts/Hiragino Sans GB.ttc"

[[ -f "$CHARSET" ]] || { echo "missing $CHARSET" >&2; exit 1; }
[[ -f "$SRC_TTC" ]] || { echo "missing system font $SRC_TTC" >&2; exit 1; }

if ! command -v pyftsubset >/dev/null 2>&1; then
  VENV="${TMPDIR:-/tmp}/meotool-fonttools"
  if [[ ! -x "$VENV/bin/pyftsubset" ]]; then
    python3 -m venv "$VENV"
    "$VENV/bin/pip" install -q fonttools brotli
  fi
  PATH="$VENV/bin:$PATH"
fi

FACE="${TMPDIR:-/tmp}/hiragino-w3-meotool.ttf"
python3 - <<PY
from fontTools.ttLib import TTCollection
TTCollection("$SRC_TTC").fonts[0].save("$FACE")
print("extracted face 0 -> $FACE")
PY

mkdir -p "$(dirname "$OUT")"
pyftsubset "$FACE" \
  --text-file="$CHARSET" \
  --output-file="$OUT" \
  --layout-features='' \
  --no-layout-closure \
  --notdef-glyph \
  --notdef-outline \
  --recommended-glyphs \
  --name-IDs='*' \
  --name-legacy \
  --name-languages='*' \
  --drop-tables+=DSIG

ls -lh "$OUT"
