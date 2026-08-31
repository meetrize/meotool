#!/usr/bin/env bash
# MeoTool 开发快速编译运行（dev profile + 增量编译）
# 用法:
#   ./scripts/dev-run.sh
#   ./scripts/dev-run.sh -- --config /path/to/config.toml
#   RUST_LOG=debug ./scripts/dev-run.sh
#
# 传给 meotool 的参数请放在 -- 之后（cargo run 约定）。

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

command -v cargo >/dev/null 2>&1 || {
    echo "error: 未找到 cargo，请先安装 Rust: https://rustup.rs" >&2
    exit 1
}

export CARGO_INCREMENTAL=1
export RUST_LOG="${RUST_LOG:-info}"

# 减少 debug 信息，加快编译与链接（未自定义 RUSTFLAGS 时生效）
if [[ -z "${RUSTFLAGS:-}" ]]; then
    export RUSTFLAGS="-C debuginfo=1"
fi

exec cargo run -- "$@"
