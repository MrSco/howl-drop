#!/usr/bin/env python3
"""Draw the add-on's icon.png (128x128) and logo.png (250x100), the images
Home Assistant shows for it: an owl's face in an AirDrop-blue drop.

  python tools/make-icons.py      writes addon/icon.png and addon/logo.png

Needs Pillow. Drawn at 8x and scaled down, for smooth edges.
"""
import math
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
S = 8  # supersampling

BLUE_TOP = (64, 156, 255)
BLUE_BOTTOM = (16, 92, 214)
WHITE = (255, 255, 255)
PUPIL = (17, 30, 60)
BEAK = (255, 176, 46)


def drop_polygon(cx, cy, r, tip_y, steps=180):
    """A drop: a circle of radius r at (cx, cy) and the two tangents from the
    tip at (cx, tip_y) above it."""
    d = cy - tip_y
    a = math.asin(r / d)              # half-angle at the tip
    # Tangent points on the circle, measured from straight up.
    t = math.pi / 2 - a
    start = -math.pi / 2 + t          # right tangent, angles in screen coords
    end = -math.pi / 2 - t + 2 * math.pi
    pts = [(cx, tip_y)]
    for i in range(steps + 1):
        ang = start + (end - start) * i / steps
        pts.append((cx + r * math.cos(ang), cy + r * math.sin(ang)))
    return pts


def owl_drop(size):
    """The mark on a transparent square of `size` pixels."""
    n = size * S
    img = Image.new('RGBA', (n, n), (0, 0, 0, 0))
    # Gradient, masked by the drop.
    grad = Image.new('RGBA', (n, n))
    gd = ImageDraw.Draw(grad)
    for y in range(n):
        f = y / (n - 1)
        gd.line([(0, y), (n, y)], fill=tuple(int(a + (b - a) * f) for a, b in zip(BLUE_TOP, BLUE_BOTTOM)) + (255,))
    mask = Image.new('L', (n, n), 0)
    cx, cy, r = n * 0.5, n * 0.6, n * 0.36
    ImageDraw.Draw(mask).polygon(drop_polygon(cx, cy, r, n * 0.04), fill=255)
    img.paste(grad, (0, 0), mask)

    d = ImageDraw.Draw(img)
    # Eyes: white, with pupils looking slightly up, toward the tip.
    eye_r, pupil_r = r * 0.30, r * 0.14
    for ex in (cx - r * 0.38, cx + r * 0.38):
        ey = cy - r * 0.08
        d.ellipse([ex - eye_r, ey - eye_r, ex + eye_r, ey + eye_r], fill=WHITE)
        py = ey - eye_r * 0.12
        d.ellipse([ex - pupil_r, py - pupil_r, ex + pupil_r, py + pupil_r], fill=PUPIL)
    # Beak, between and below the eyes.
    by = cy + r * 0.18
    d.polygon([(cx - r * 0.12, by), (cx + r * 0.12, by), (cx, by + r * 0.24)], fill=BEAK)
    return img.resize((size, size), Image.LANCZOS)


def font(px):
    for name in ('segoeuib.ttf', 'arialbd.ttf', 'DejaVuSans-Bold.ttf'):
        for base in (Path('C:/Windows/Fonts'), Path('/usr/share/fonts/truetype/dejavu')):
            if (base / name).exists():
                return ImageFont.truetype(str(base / name), px)
    return ImageFont.load_default()


def logo():
    w, h = 250, 100
    img = Image.new('RGBA', (w * S, h * S), (0, 0, 0, 0))
    mark = owl_drop(84).resize((84 * S, 84 * S), Image.LANCZOS)
    img.paste(mark, (0, 8 * S), mark)
    d = ImageDraw.Draw(img)
    f = font(30 * S)
    d.text((88 * S, h * S / 2), 'Howl', font=f, fill=BLUE_BOTTOM + (255,), anchor='lm')
    howl_w = d.textlength('Howl', font=f)
    d.text((88 * S + howl_w, h * S / 2), 'Drop', font=f, fill=(90, 100, 120, 255), anchor='lm')
    return img.resize((w, h), Image.LANCZOS)


def main():
    out = ROOT / 'addon'
    owl_drop(128).save(out / 'icon.png', optimize=True)
    logo().save(out / 'logo.png', optimize=True)
    print(f'wrote {out / "icon.png"} and {out / "logo.png"}')


if __name__ == '__main__':
    main()
