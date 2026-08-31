#!/usr/bin/env bash
# MeoTool 内存基线采样（Release）
#
# 用法：
#   ./scripts/memory-baseline.sh
#   ./scripts/memory-baseline.sh --seconds 300 --interval 5
#   ./scripts/memory-baseline.sh --bin /path/to/meotool
#
# 输出：每隔 interval 秒打印 phys_footprint / RSS（MiB），便于对照改造前后。

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SECONDS_TOTAL=120
INTERVAL=5
BIN=""
CONFIG=""

usage() {
    cat <<'EOF'
MeoTool Release 内存基线采样

选项：
  --seconds <n>     采样总时长（默认 120）
  --interval <n>    采样间隔秒（默认 5）
  --bin <path>      指定二进制（默认 target/release/meotool）
  --config <path>   传给 meotool 的 --config
  -h, --help
EOF
}

die() { printf 'error: %s\n' "$*" >&2; exit 1; }
log() { printf '==> %s\n' "$*" >&2; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --seconds) SECONDS_TOTAL="$2"; shift 2 ;;
        --interval) INTERVAL="$2"; shift 2 ;;
        --bin) BIN="$2"; shift 2 ;;
        --config) CONFIG="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "未知参数: $1" ;;
    esac
done

if [[ -z "$BIN" ]]; then
    BIN="$ROOT_DIR/target/release/meotool"
fi

if [[ ! -x "$BIN" ]]; then
    log "构建 release…"
    (cd "$ROOT_DIR" && cargo build --release)
fi
[[ -x "$BIN" ]] || die "找不到可执行文件: $BIN"

ARGS=()
if [[ -n "$CONFIG" ]]; then
    ARGS+=(--config "$CONFIG")
fi

export RUST_LOG="${RUST_LOG:-meotool=info,info}"

log "启动: $BIN ${ARGS[*]+${ARGS[*]}}"
"$BIN" ${ARGS[@]+"${ARGS[@]}"} &
PID=$!
trap 'kill "$PID" 2>/dev/null || true' EXIT

sleep 2
if ! kill -0 "$PID" 2>/dev/null; then
    die "进程启动失败"
fi

printf 'time_s\tphys_mb\trss_mb\n'
elapsed=0
while [[ "$elapsed" -le "$SECONDS_TOTAL" ]]; do
    if ! kill -0 "$PID" 2>/dev/null; then
        die "进程已退出"
    fi
    # footprint(1) 在较新 macOS 可用；失败则回退 ps
    phys=""
    if command -v footprint >/dev/null 2>&1; then
        phys="$(footprint -p "$PID" 2>/dev/null | awk '/phys_footprint/ {print $2; exit}')"
    fi
    rss_kb="$(ps -o rss= -p "$PID" | tr -d ' ')"
    rss_mb="$(python3 -c "print(round(int('${rss_kb:-0}')/1024, 1))")"
    if [[ -n "$phys" ]]; then
        phys_mb="$(python3 -c "import re; s='${phys}';
m=re.match(r'([0-9.]+)([KMG]?)', s);
v=float(m.group(1)); u=m.group(2);
print(round(v/(1024 if u=='G' else 1/1024 if u=='K' else 1),1) if u!='G' else round(v*1024,1) if False else (round(v*1024,1) if u=='G' else round(v/1024,1) if u=='K' else round(v,1)))" 2>/dev/null || echo "$phys")"
    else
        phys_mb="?"
    fi
    printf '%s\t%s\t%s\n' "$elapsed" "$phys_mb" "$rss_mb"
    sleep "$INTERVAL"
    elapsed=$((elapsed + INTERVAL))
done

log "采样结束，已停止进程"
