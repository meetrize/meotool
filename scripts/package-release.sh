#!/usr/bin/env bash
# MeoTool 发布打包脚本（macOS）
#
# 产出物（默认在 dist/ 目录）：
#   MeoTool-<version>-macos-<arch>/
#     MeoTool.app
#     README.txt
#   MeoTool-<version>-macos-<arch>.zip
#   MeoTool-<version>-macos-<arch>.dmg   # 加 --dmg 时
#
# 用法：
#   ./scripts/package-release.sh
#   ./scripts/package-release.sh --universal
#   ./scripts/package-release.sh --dmg
#   ./scripts/package-release.sh --arch x86_64-apple-darwin

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

APP_NAME="MeoTool"
BINARY_NAME="meotool"
BUNDLE_ID="com.meotool.app"
MIN_MACOS="12.0"

ARCH=""
UNIVERSAL=0
BUILD_DMG=0
SKIP_BUILD=0
CLEAN=0

usage() {
    cat <<'EOF'
MeoTool 发布打包脚本

选项：
  --arch <triple>   指定 Rust target（默认：当前主机）
  --universal       构建 arm64 + x86_64 通用二进制（仅 macOS）
  --dmg             额外生成 .dmg 磁盘镜像
  --skip-build      跳过 cargo build，使用已有 release 产物
  --clean           打包前清理 dist/ 与 release 构建缓存
  -h, --help        显示帮助

示例：
  ./scripts/package-release.sh
  ./scripts/package-release.sh --universal --dmg
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
        --arch)
            [[ $# -ge 2 ]] || die "--arch 需要参数"
            ARCH="$2"
            shift 2
            ;;
        --universal)
            UNIVERSAL=1
            shift
            ;;
        --dmg)
            BUILD_DMG=1
            shift
            ;;
        --skip-build)
            SKIP_BUILD=1
            shift
            ;;
        --clean)
            CLEAN=1
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
    die "此脚本仅支持在 macOS 上打包 .app 发布包"
fi

command -v cargo >/dev/null 2>&1 || die "未找到 cargo，请先安装 Rust"
command -v python3 >/dev/null 2>&1 || die "未找到 python3（用于解析版本号）"

read_version() {
    python3 - "$ROOT_DIR/Cargo.toml" <<'PY'
import pathlib
import re
import sys

text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE)
if not match:
    raise SystemExit("无法在 Cargo.toml 中找到 version")
print(match.group(1))
PY
}

VERSION="$(read_version)"
HOST_ARCH="$(uname -m)"

if [[ "$UNIVERSAL" -eq 1 ]]; then
    PACKAGE_ARCH="universal"
    TARGETS=("aarch64-apple-darwin" "x86_64-apple-darwin")
elif [[ -n "$ARCH" ]]; then
    PACKAGE_ARCH="${ARCH%%-*}"
    TARGETS=("$ARCH")
else
  case "$HOST_ARCH" in
    arm64) PACKAGE_ARCH="arm64" ;;
    x86_64) PACKAGE_ARCH="x86_64" ;;
    *) PACKAGE_ARCH="$HOST_ARCH" ;;
  esac
    TARGETS=("")
fi

DIST_DIR="$ROOT_DIR/dist"
STAGING_NAME="${APP_NAME}-${VERSION}-macos-${PACKAGE_ARCH}"
STAGING_DIR="$DIST_DIR/$STAGING_NAME"
APP_DIR="$STAGING_DIR/${APP_NAME}.app"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"

if [[ "$CLEAN" -eq 1 ]]; then
    log "清理 dist/ 与 release 构建"
    rm -rf "$DIST_DIR"
    cargo clean --release
fi

mkdir -p "$DIST_DIR"

build_target() {
    local target="$1"
    if [[ -z "$target" ]]; then
        log "构建 release（本机）"
        cargo build --release
        echo "$ROOT_DIR/target/release/$BINARY_NAME"
        return
    fi

    log "构建 release target=$target"
    rustup target add "$target" >/dev/null 2>&1 || true
    cargo build --release --target "$target"
    echo "$ROOT_DIR/target/$target/release/$BINARY_NAME"
}

if [[ "$SKIP_BUILD" -eq 0 ]]; then
    if [[ "$UNIVERSAL" -eq 1 ]]; then
        BIN_ARM="$(build_target "${TARGETS[0]}")"
        BIN_X86="$(build_target "${TARGETS[1]}")"
        UNIVERSAL_BIN="$DIST_DIR/.build/${BINARY_NAME}-universal"
        mkdir -p "$(dirname "$UNIVERSAL_BIN")"
        log "合并通用二进制 (lipo)"
        lipo -create "$BIN_ARM" "$BIN_X86" -output "$UNIVERSAL_BIN"
        RELEASE_BIN="$UNIVERSAL_BIN"
    else
        RELEASE_BIN="$(build_target "${TARGETS[0]}")"
    fi
else
    if [[ "$UNIVERSAL" -eq 1 ]]; then
        RELEASE_BIN="$DIST_DIR/.build/${BINARY_NAME}-universal"
    elif [[ -n "${TARGETS[0]}" ]]; then
        RELEASE_BIN="$ROOT_DIR/target/${TARGETS[0]}/release/$BINARY_NAME"
    else
        RELEASE_BIN="$ROOT_DIR/target/release/$BINARY_NAME"
    fi
    [[ -f "$RELEASE_BIN" ]] || die "未找到 release 二进制: $RELEASE_BIN"
    log "跳过构建，使用 $RELEASE_BIN"
fi

log "准备目录结构"
rm -rf "$STAGING_DIR"
mkdir -p "$MACOS_DIR/assets/icons" "$MACOS_DIR/assets/fonts" "$MACOS_DIR/configs" "$RESOURCES_DIR/configs"

log "复制二进制与资源"
install -m 755 "$RELEASE_BIN" "$MACOS_DIR/$BINARY_NAME"

# 仅打包核心图标 + 配置引用，避免把 5000+ Tabler 图标打进 .app（磁盘与选择器内存）
CORE_ICONS_FILE="$ROOT_DIR/scripts/core-icons.txt"
if [[ -f "$CORE_ICONS_FILE" ]]; then
    log "按 core-icons.txt 复制精简图标集"
    while IFS= read -r line || [[ -n "$line" ]]; do
        [[ -z "$line" || "$line" == \#* ]] && continue
        src="$ROOT_DIR/assets/icons/$line"
        if [[ -f "$src" ]]; then
            cp "$src" "$MACOS_DIR/assets/icons/$line"
        else
            log "warn: 核心图标缺失 $line"
        fi
    done <"$CORE_ICONS_FILE"
    # 配置中引用但未列入清单的图标补拷
    if [[ -f "$ROOT_DIR/configs/config.toml" ]]; then
        python3 - "$ROOT_DIR/configs/config.toml" "$ROOT_DIR/assets/icons" "$MACOS_DIR/assets/icons" <<'PY'
import re, sys, pathlib, shutil
cfg, src_dir, dst_dir = map(pathlib.Path, sys.argv[1:])
text = cfg.read_text(encoding="utf-8")
names = set(re.findall(r'(?:icon|collapsed_icon)\s*=\s*"([^"]+)"', text))
for name in names:
    s = src_dir / name
    d = dst_dir / name
    if s.is_file() and not d.exists():
        shutil.copy2(s, d)
        print(f"copied config icon {name}", file=sys.stderr)
PY
    fi
    ICON_COUNT="$(find "$MACOS_DIR/assets/icons" -type f -name '*.png' | wc -l | tr -d ' ')"
    log "已打包图标数量: $ICON_COUNT"
else
    log "未找到 core-icons.txt，回退复制全部 icons/"
    cp -R "$ROOT_DIR/assets/icons/." "$MACOS_DIR/assets/icons/"
fi

if [[ -d "$ROOT_DIR/assets/fonts" ]]; then
    cp -R "$ROOT_DIR/assets/fonts/." "$MACOS_DIR/assets/fonts/"
fi
cp "$ROOT_DIR/configs/config.toml" "$MACOS_DIR/configs/config.toml"
cp "$ROOT_DIR/configs/config.toml" "$RESOURCES_DIR/configs/config.toml"

APP_ICNS="$ROOT_DIR/assets/AppIcon.icns"
if [[ ! -f "$APP_ICNS" ]]; then
    log "未找到 AppIcon.icns，尝试生成"
    "$ROOT_DIR/scripts/generate-app-icon.sh"
fi
[[ -f "$APP_ICNS" ]] || die "缺少应用图标: $APP_ICNS"
cp "$APP_ICNS" "$RESOURCES_DIR/AppIcon.icns"

if [[ -f "$ROOT_DIR/LICENSE" ]]; then
    cp "$ROOT_DIR/LICENSE" "$RESOURCES_DIR/LICENSE"
fi

log "生成 Info.plist"
cat >"$CONTENTS_DIR/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>zh-Hans</string>
    <key>CFBundleExecutable</key>
    <string>${BINARY_NAME}</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>LSMinimumSystemVersion</key>
    <string>${MIN_MACOS}</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.productivity</string>
</dict>
</plist>
PLIST

log "生成 README"
cat >"$STAGING_DIR/README.txt" <<EOF
MeoTool ${VERSION} (macOS ${PACKAGE_ARCH})
================================

安装
----
1. 将 MeoTool.app 拖入「应用程序」文件夹
2. 首次打开若被 Gatekeeper 拦截：
   右键 MeoTool.app → 打开 → 再次确认打开

系统权限（必须）
----------------
首次运行请在系统设置中授予：
- 隐私与安全性 → 辅助功能
- 隐私与安全性 → 输入监控

配置文件
--------
默认读取：
  ~/Library/Application Support/meotool/config.toml

应用包内自带默认配置：
  MeoTool.app/Contents/MacOS/configs/config.toml

也可手动指定：
  /Applications/MeoTool.app/Contents/MacOS/meotool --config /path/to/config.toml

图标资源
--------
  MeoTool.app/Contents/MacOS/assets/icons/

命令行运行（调试）
------------------
  open -a MeoTool
  # 或
  /Applications/MeoTool.app/Contents/MacOS/meotool

构建信息
--------
  架构: ${PACKAGE_ARCH}
  版本: ${VERSION}
  构建时间: $(date '+%Y-%m-%d %H:%M:%S %Z')
EOF

ZIP_PATH="$DIST_DIR/${STAGING_NAME}.zip"
log "打包 ZIP: $ZIP_PATH"
rm -f "$ZIP_PATH"
ditto -c -k --sequesterRsrc --keepParent "$STAGING_DIR" "$ZIP_PATH"

if [[ "$BUILD_DMG" -eq 1 ]]; then
    DMG_PATH="$DIST_DIR/${STAGING_NAME}.dmg"
    DMG_TMP="$DIST_DIR/.${STAGING_NAME}.dmg.tmp"
    log "生成 DMG: $DMG_PATH"
    rm -f "$DMG_TMP" "$DMG_PATH"
    hdiutil create \
        -volname "$APP_NAME" \
        -srcfolder "$STAGING_DIR" \
        -ov \
        -format UDZO \
        "$DMG_TMP"
    mv "$DMG_TMP" "$DMG_PATH"
fi

log "校验应用包"
/usr/bin/codesign --verify --deep --strict "$APP_DIR" >/dev/null 2>&1 || true
file "$MACOS_DIR/$BINARY_NAME"

cat <<EOF

打包完成。

目录 : $STAGING_DIR
ZIP  : $ZIP_PATH
EOF

if [[ "$BUILD_DMG" -eq 1 ]]; then
    echo "DMG  : $DMG_PATH"
fi

echo
echo "可选：本地 ad-hoc 签名（减轻 Gatekeeper 提示）"
echo "  codesign --force --deep --sign - \"$APP_DIR\""
