#!/usr/bin/env python3
"""Make the add-on's icon.png and logo.png, the images Home Assistant shows
for it, from the artwork in art/howldrop.png.

  python tools/make-icons.py [SOURCE]

SOURCE (default art/howldrop.png) is a square app icon on a black field. The
black outside its rounded square becomes transparent: it is filled from the
corners, so dark blue inside the icon is never touched. Without an argument
the stored artwork is used; with one, that file also replaces it (scaled to
512 px). Needs Pillow.
"""
import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
ART = ROOT / 'art' / 'howldrop.png'
BLUE = (24, 110, 230)
GREY = (90, 100, 120)


def transparent_corners(img, threshold=24):
    """Make the near-black field around the icon transparent."""
    img = img.convert('RGBA')
    w, h = img.size
    # Pixels close to black, then only the ones reachable from a corner.
    dark = img.convert('L').point(lambda v: 255 if v <= threshold else 0)
    field = Image.new('L', img.size, 0)
    for corner in ((0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)):
        if dark.getpixel(corner):
            region = dark.copy()
            ImageDraw.floodfill(region, corner, 128)
            field = ImageChops.lighter(field, region.point(lambda v: 255 if v == 128 else 0))
    alpha = ImageChops.subtract(img.getchannel('A'), field)
    img.putalpha(alpha)
    return img


def font(px):
    for name in ('segoeuib.ttf', 'arialbd.ttf', 'DejaVuSans-Bold.ttf'):
        for base in (Path('C:/Windows/Fonts'), Path('/usr/share/fonts/truetype/dejavu')):
            if (base / name).exists():
                return ImageFont.truetype(str(base / name), px)
    return ImageFont.load_default()


def logo(icon):
    s = 4
    w, h = 250, 100
    img = Image.new('RGBA', (w * s, h * s), (0, 0, 0, 0))
    mark = icon.resize((88 * s, 88 * s), Image.LANCZOS)
    img.paste(mark, (0, 6 * s), mark)
    d = ImageDraw.Draw(img)
    f = font(30 * s)
    d.text((94 * s, h * s / 2), 'Howl', font=f, fill=BLUE + (255,), anchor='lm')
    d.text((94 * s + d.textlength('Howl', font=f), h * s / 2), 'Drop', font=f, fill=GREY + (255,), anchor='lm')
    return img.resize((w, h), Image.LANCZOS)


def main(argv):
    if argv:
        src = Image.open(argv[0])
        ART.parent.mkdir(exist_ok=True)
        transparent_corners(src).resize((512, 512), Image.LANCZOS).save(ART, optimize=True)
    icon = Image.open(ART).convert('RGBA')
    out = ROOT / 'addon'
    icon.resize((256, 256), Image.LANCZOS).save(out / 'icon.png', optimize=True)
    logo(icon).save(out / 'logo.png', optimize=True)
    print(f'wrote {out / "icon.png"} and {out / "logo.png"} from {ART}')


if __name__ == '__main__':
    main(sys.argv[1:])
