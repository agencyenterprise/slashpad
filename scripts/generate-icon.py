#!/usr/bin/env python3
"""Regenerate Slashpad app icons (white slash on dark squircle).

Run: python3 scripts/generate-icon.py
"""
from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

REPO_ROOT = Path(__file__).resolve().parent.parent
ICONS_DIR = REPO_ROOT / "icons"

BG = (11, 11, 13, 255)
FG = (255, 255, 255, 255)

BODY_FILL = 0.824
SLASH_WIDTH = 0.15
SLASH_PAD = 0.11
SLASH_TILT_DEG = 32
SUPERELLIPSE_N = 5.0
SUPERCANVAS = 2048


def superellipse_mask(size: int, n: float = SUPERELLIPSE_N) -> Image.Image:
    coords = (np.arange(size) - size / 2 + 0.5) / (size / 2)
    xs, ys = np.meshgrid(coords, coords)
    inside = (np.abs(xs) ** n + np.abs(ys) ** n) <= 1.0
    return Image.fromarray((inside * 255).astype(np.uint8), mode="L")


def render_master(size: int = SUPERCANVAS) -> Image.Image:
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))

    body_px = int(size * BODY_FILL)
    body_offset = (size - body_px) // 2
    body = Image.new("RGBA", (body_px, body_px), (0, 0, 0, 0))
    body.paste(BG, (0, 0, body_px, body_px), superellipse_mask(body_px))
    canvas.paste(body, (body_offset, body_offset), body)

    stroke = int(size * SLASH_WIDTH)
    total_h = int(body_px * (1 - 2 * SLASH_PAD))
    slash_layer = Image.new("RGBA", (body_px, body_px), (0, 0, 0, 0))
    draw = ImageDraw.Draw(slash_layer)
    cx = body_px // 2
    cy = body_px // 2
    x0, x1 = cx - stroke // 2, cx + stroke // 2
    y0, y1 = cy - total_h // 2, cy + total_h // 2
    draw.rounded_rectangle([x0, y0, x1, y1], radius=stroke // 2, fill=FG)
    slash_layer = slash_layer.rotate(-SLASH_TILT_DEG, resample=Image.BICUBIC)

    canvas.paste(slash_layer, (body_offset, body_offset), slash_layer)
    return canvas


def downsample(img: Image.Image, size: int) -> Image.Image:
    return img.resize((size, size), Image.LANCZOS)


def build_iconset(master: Image.Image, dest: Path) -> None:
    dest.mkdir(parents=True, exist_ok=True)
    specs = [
        ("icon_16x16.png", 16),
        ("icon_16x16@2x.png", 32),
        ("icon_32x32.png", 32),
        ("icon_32x32@2x.png", 64),
        ("icon_128x128.png", 128),
        ("icon_128x128@2x.png", 256),
        ("icon_256x256.png", 256),
        ("icon_256x256@2x.png", 512),
        ("icon_512x512.png", 512),
        ("icon_512x512@2x.png", 1024),
    ]
    for name, size in specs:
        downsample(master, size).save(dest / name, "PNG")


def main() -> int:
    master = render_master(SUPERCANVAS)
    master_1024 = downsample(master, 1024)
    master_512 = downsample(master, 512)

    master_1024.save(ICONS_DIR / "icon-1024.png", "PNG")
    master_512.save(ICONS_DIR / "icon.png", "PNG")
    for s in (32, 64, 128):
        downsample(master, s).save(ICONS_DIR / f"{s}x{s}.png", "PNG")
    downsample(master, 256).save(ICONS_DIR / "128x128@2x.png", "PNG")

    iconset = ICONS_DIR / "AppIcon.iconset"
    if iconset.exists():
        shutil.rmtree(iconset)
    build_iconset(master, iconset)
    subprocess.run(
        ["iconutil", "-c", "icns", str(iconset), "-o", str(ICONS_DIR / "icon.icns")],
        check=True,
    )
    shutil.rmtree(iconset)

    master_1024.save(
        ICONS_DIR / "icon.ico",
        format="ICO",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )

    print(f"Regenerated icons in {ICONS_DIR}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
