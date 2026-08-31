#!/usr/bin/env bash
# MeoTool 一键打包并安装到系统「应用程序」目录
#
# 流程：release 打包 → ad-hoc 签名 → 安装到 /Applications →（可选）启动
#
# 用法：
#   ./scripts/install-to-applications.sh
#   ./scripts/install-to-applications.sh --universal
#   ./scripts/install-to-applications.sh --skip-build
#   ./scripts/install-to-applications.sh --no-launch
#   ./scripts/install-to-applications.sh --dest ~/Applications

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

APP_NAME="MeoTool"
INSTALL_DEST="/Applications"
NO_LAUNCH=0
PACKAGE_ARGS=()

usage() {
    cat <<'EOF'
MeoTool 一键打包并安装到「应用程序」

选项：
  --dest <dir>      安装目录（默认：/Applications）
  --no-launch       安装后不自动启动
  --arch <triple>   传给 package-release.sh
  --universal       构建通用二进制
  --skip-build      跳过 cargo build，使用已有 release 产物
  --clean           打包前清理 dist/ 与 release 构建缓存
  -h, --help        显示帮助

示例：
  ./scripts/install-to-applications.sh
  ./scripts/install-to-applications.sh --universal --no-launch
  ./scripts/install-to-applications.sh --dest "$HOME/Applications"
EOF
}

log() {
    printf '==> %s\n' "$*" >&2
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dest)
            [[ $# -ge 2 ]] || die "--dest 需要参数"
            INSTALL_DEST="$2"
            shift 2
            ;;
        --no-launch)
            NO_LAUNCH=1
            shift
            ;;
        --arch)
            [[ $# -ge 2 ]] || die "--arch 需要参数"
            PACKAGE_ARGS+=(--arch "$2")
            shift 2
            ;;
        --universal | --skip-build | --clean)
            PACKAGE_ARGS+=("$1")
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            die "未知参数: $1（使用 --help 查看用法）"
            ;;
    esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
    die "此脚本仅支持 macOS"
fi

[[ -x "$ROOT_DIR/scripts/package-release.sh" ]] \
    || die "未找到可执行的 scripts/package-release.sh"

log "开始打包"
"$ROOT_DIR/scripts/package-release.sh" "${PACKAGE_ARGS[@]+"${PACKAGE_ARGS[@]}"}"

APP_SRC=""
while IFS= read -r -d '' candidate; do
    if [[ -z "$APP_SRC" || "$candidate" -nt "$APP_SRC" ]]; then
        APP_SRC="$candidate"
    fi
done < <(find "$ROOT_DIR/dist" -maxdepth 2 -type d -name "${APP_NAME}.app" -print0 2>/dev/null)
[[ -n "$APP_SRC" && -d "$APP_SRC" ]] || die "打包后未找到 ${APP_NAME}.app"

APP_DEST="${INSTALL_DEST%/}/${APP_NAME}.app"

log "ad-hoc 签名: $APP_SRC"
codesign --force --deep --sign - "$APP_SRC" >/dev/null

# 替换安装前先退出正在运行的实例，避免覆盖失败
if pgrep -xq "$APP_NAME" || pgrep -xq meotool; then
    log "退出正在运行的 ${APP_NAME}"
    osascript -e "tell application \"${APP_NAME}\" to quit" >/dev/null 2>&1 || true
    # 等待进程退出；超时后强制结束
    for _ in $(seq 1 20); do
        if ! pgrep -xq "$APP_NAME" && ! pgrep -xq meotool; then
            break
        fi
        sleep 0.25
    done
    if pgrep -xq "$APP_NAME" || pgrep -xq meotool; then
        log "进程未及时退出，强制结束"
        pkill -x "$APP_NAME" 2>/dev/null || true
        pkill -x meotool 2>/dev/null || true
        sleep 0.5
    fi
fi

run_as_needed() {
    if [[ -w "$INSTALL_DEST" ]] && { [[ ! -e "$APP_DEST" ]] || [[ -w "$APP_DEST" ]]; }; then
        "$@"
    else
        log "需要管理员权限写入 $INSTALL_DEST"
        sudo "$@"
    fi
}

mkdir -p "$INSTALL_DEST" 2>/dev/null || sudo mkdir -p "$INSTALL_DEST"

log "安装到 $APP_DEST"
run_as_needed rm -rf "$APP_DEST"
run_as_needed ditto "$APP_SRC" "$APP_DEST"
run_as_needed xattr -cr "$APP_DEST" 2>/dev/null || true

# 刷新 Launch Services / 图标缓存，避免 Dock 仍显示旧空白图标
if [[ -d "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support" ]]; then
    LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
    if [[ -x "$LSREGISTER" ]]; then
        "$LSREGISTER" -f "$APP_DEST" >/dev/null 2>&1 || true
    fi
fi

log "校验安装结果"
[[ -x "$APP_DEST/Contents/MacOS/meotool" ]] || die "安装失败：缺少可执行文件"
[[ -f "$APP_DEST/Contents/Resources/AppIcon.icns" ]] || log "警告: 未找到 AppIcon.icns，Finder 可能无图标"

cat <<EOF

安装完成。

应用包 : $APP_DEST
来源   : $APP_SRC
EOF

if [[ "$NO_LAUNCH" -eq 0 ]]; then
    log "启动 ${APP_NAME}"
    open -a "$APP_DEST"
    echo
    echo "已启动。若被 Gatekeeper 拦截：右键应用 → 打开。"
else
    echo
    echo "启动：open -a ${APP_NAME}"
fi
