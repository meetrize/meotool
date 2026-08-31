#!/usr/bin/env python3
"""Generate macOS AppIcon.icns from assets/icons/pet-cat.png."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFilter
except ImportError:
    print("error: 需要 Pillow（pip install pillow）", file=sys.stderr)
    sys.exit(1)

ROOT = Path(__file__).resolve().parents[1]
SRC_ICON = ROOT / "assets" / "icons" / "pet-cat.png"
MASTER = ROOT / "assets" / "AppIcon-1024.png"
ICNS = ROOT / "assets" / "AppIcon.icns"

# iconutil 需要的文件名（避免在 shell 中直接写 @）
ICON_ENTRIES = [
    (16, "icon_16x16.png"),
    (32, "icon_16x16{scale}.png"),
    (32, "icon_32x32.png"),
    (64, "icon_32x32{scale}.png"),
    (128, "icon_128x128.png"),
    (256, "icon_128x128{scale}.png"),
    (256, "icon_256x256.png"),
    (512, "icon_256x256{scale}.png"),
    (512, "icon_512x512.png"),
    (1024, "icon_512x512{scale}.png"),
]
SCALE_SUFFIX = "@" + "2x"


def compose_master(src_path: Path, out_path: Path) -> None:
    """合成铺满画布的不透明应用图标（无 alpha，避免被系统嵌进通用底板）。"""
    src = Image.open(src_path).convert("RGBA")
    size = 1024
    bg_top = (255, 236, 220)
    bg_bottom = (255, 214, 186)

    # 使用 RGB（无 alpha）：macOS Icon Services 对带透明通道的图
    # 往往会缩成「圆角底板里的小正方形」，而不是铺满圆角矩形。
    canvas = Image.new("RGB", (size, size), bg_top)
    draw = ImageDraw.Draw(canvas)
    for y in range(size):
        t = y / (size - 1)
        r = int(bg_top[0] * (1 - t) + bg_bottom[0] * t)
        g = int(bg_top[1] * (1 - t) + bg_bottom[1] * t)
        b = int(bg_top[2] * (1 - t) + bg_bottom[2] * t)
        draw.line([(0, y), (size, y)], fill=(r, g, b))

    # 轻微高光（同样无 alpha，直接叠色）
    highlight = Image.new("RGB", (size, size), (0, 0, 0))
    hd = ImageDraw.Draw(highlight)
    hd.ellipse([80, 40, 944, 520], fill=(255, 255, 255))
    highlight = highlight.filter(ImageFilter.GaussianBlur(60))
    canvas = Image.blend(canvas, highlight, 0.08)

    # 猫咪尽量占满安全区，贴近圆角矩形视觉中心
    max_w = int(size * 0.92)
    max_h = int(size * 0.84)
    scale = min(max_w / src.width, max_h / src.height)
    new_w = int(src.width * scale)
    new_h = int(src.height * scale)
    cat = src.resize((new_w, new_h), Image.Resampling.LANCZOS)

    # 阴影：在 RGB 画布上用加深混合近似
    shadow_layer = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    shadow_cat = Image.new("RGBA", cat.size, (90, 45, 25, 80))
    shadow_cat.putalpha(cat.split()[3])
    sx = (size - new_w) // 2 + 6
    sy = (size - new_h) // 2 + 36
    shadow_layer.paste(shadow_cat, (sx, sy), shadow_cat)
    shadow_layer = shadow_layer.filter(ImageFilter.GaussianBlur(16))
    canvas = Image.alpha_composite(canvas.convert("RGBA"), shadow_layer).convert("RGB")

    x = (size - new_w) // 2
    y = (size - new_h) // 2 + 28
    # 贴到 RGB：先转 RGBA 合成再压回 RGB
    layer = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    layer.paste(cat, (x, y), cat)
    canvas = Image.alpha_composite(canvas.convert("RGBA"), layer).convert("RGB")

    out_path.parent.mkdir(parents=True, exist_ok=True)
    # RGB PNG，本身无 alpha
    canvas.save(out_path, "PNG")


def build_icns(master: Path, icns: Path) -> None:
    work = Path(tempfile.mkdtemp(prefix="meotool-icon-"))
    try:
        flat = work / "flat"
        flat.mkdir()
        sizes = sorted({px for px, _ in ICON_ENTRIES})
        for px in sizes:
            out = flat / f"{px}.png"
            subprocess.run(
                ["sips", "-z", str(px), str(px), str(master), "--out", str(out)],
                check=True,
                capture_output=True,
            )
            # 经 BMP 中转去掉 alpha（sips 对 hasAlpha 属性不可直接写）
            bmp = flat / f"{px}.bmp"
            subprocess.run(
                ["sips", "-s", "format", "bmp", str(out), "--out", str(bmp)],
                check=True,
                capture_output=True,
            )
            subprocess.run(
                ["sips", "-s", "format", "png", str(bmp), "--out", str(out)],
                check=True,
                capture_output=True,
            )
            bmp.unlink(missing_ok=True)

        # 先在普通目录组装，最后再改名为 .iconset
        staging = work / "icons"
        staging.mkdir()
        for px, pattern in ICON_ENTRIES:
            name = pattern.format(scale=SCALE_SUFFIX)
            shutil.copy2(flat / f"{px}.png", staging / name)

        names = sorted(os.listdir(staging))
        if len(names) != 10:
            raise SystemExit(f"iconset 文件数异常: {len(names)} -> {names}")

        iconset = work / "AppIcon.iconset"
        staging.rename(iconset)
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(icns)],
            check=True,
        )
    finally:
        shutil.rmtree(work, ignore_errors=True)


def main() -> None:
    if not SRC_ICON.is_file():
        raise SystemExit(f"未找到源图标: {SRC_ICON}")
    print(f"==> 合成 {MASTER}")
    compose_master(SRC_ICON, MASTER)
    print(f"==> 生成 {ICNS}")
    build_icns(MASTER, ICNS)
    print("完成")
    print(f"  {MASTER}")
    print(f"  {ICNS}")


if __name__ == "__main__":
    main()
