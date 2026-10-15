#!/usr/bin/env python3
"""Deterministic generator of the original sample art of `lluvia-de-neon`.

The four sources are drawn only from code: gradients, rectangles, ellipses
and polygons, with no external image, font, photograph or model. Running it
again with the same Pillow major version reproduces the same pictures.

Usage: python3 -I generate.py <output-directory>
"""

import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

BACKGROUND = (1280, 560)
PORTRAIT = (1280, 1280)


def gradient(size, top, bottom):
    width, height = size
    image = Image.new("RGB", size)
    draw = ImageDraw.Draw(image)
    for y in range(height):
        t = y / max(1, height - 1)
        color = tuple(round(a + (b - a) * t) for a, b in zip(top, bottom))
        draw.line([(0, y), (width, y)], fill=color)
    return image


def rain(draw, size, color, step, length, slant):
    width, height = size
    for index, x in enumerate(range(-height, width, step)):
        offset = (index * 37) % height
        for y in range(-length + offset % length, height, length * 3):
            draw.line([(x + y * slant, y), (x + (y + length) * slant, y + length)], fill=color, width=1)


def office():
    image = gradient(BACKGROUND, (14, 18, 46), (58, 16, 62))
    draw = ImageDraw.Draw(image)
    # The big window and its frame.
    draw.rectangle([120, 40, 1160, 400], fill=(20, 26, 62), outline=(70, 70, 110), width=10)
    for x in (467, 813):
        draw.line([(x, 40), (x, 400)], fill=(70, 70, 110), width=8)
    # Distant towers behind the glass.
    for index, (left, top) in enumerate([(160, 170), (300, 120), (520, 210), (690, 140), (900, 190), (1040, 110)]):
        draw.rectangle([left, top, left + 90 + (index % 3) * 20, 400], fill=(28, 30, 70))
        for y in range(top + 20, 390, 34):
            for x in range(left + 12, left + 80, 26):
                if (x + y + index) % 3:
                    draw.rectangle([x, y, x + 8, y + 12], fill=(90, 70, 120))
    # Pink neon across the street, bleeding through the rain.
    image = ImageChops.add(image, blurred(BACKGROUND, lambda d: d.rectangle([760, 90, 1080, 150], fill=(200, 40, 140)), 28))
    draw = ImageDraw.Draw(image)
    draw.rectangle([780, 105, 1060, 135], outline=(255, 120, 210), width=4)
    rain(draw, (1280, 400), (96, 110, 170), 23, 26, 0.18)
    # Desk and lamp silhouettes.
    draw.rectangle([0, 430, 1280, 560], fill=(10, 8, 20))
    draw.polygon([(180, 430), (1100, 430), (1180, 500), (100, 500)], fill=(26, 18, 34))
    draw.polygon([(900, 430), (930, 330), (980, 330), (1010, 430)], fill=(16, 12, 24))
    image = ImageChops.add(image, blurred(BACKGROUND, lambda d: d.ellipse([890, 300, 1020, 360], fill=(150, 120, 60)), 24))
    return image


def club():
    image = gradient(BACKGROUND, (40, 6, 40), (8, 4, 18))
    draw = ImageDraw.Draw(image)
    # Back wall shelves.
    for y in (120, 200, 280):
        draw.rectangle([140, y, 1140, y + 10], fill=(60, 24, 64))
        for x in range(170, 1120, 46):
            height = 30 + (x * 7) % 26
            draw.rectangle([x, y - height, x + 18, y], fill=(70 + (x % 50), 30, 90))
    # Neon sign and its light.
    image = ImageChops.add(image, blurred(BACKGROUND, lambda d: d.rounded_rectangle([430, 24, 850, 92], 30, fill=(20, 160, 200)), 26))
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle([450, 36, 830, 80], 22, outline=(140, 240, 255), width=5)
    # Bar counter and stools.
    draw.rectangle([0, 360, 1280, 560], fill=(12, 6, 16))
    draw.rectangle([60, 330, 1220, 372], fill=(46, 16, 40))
    for x in range(150, 1200, 210):
        draw.ellipse([x, 400, x + 90, 420], fill=(30, 12, 30))
        draw.rectangle([x + 40, 420, x + 50, 540], fill=(30, 12, 30))
    image = ImageChops.add(image, blurred(BACKGROUND, lambda d: d.rectangle([60, 330, 1220, 340], fill=(160, 40, 110)), 14))
    return image


def portrait(top, bottom, rim, hat):
    image = gradient(PORTRAIT, top, bottom)
    # Rim light behind the figure.
    image = ImageChops.add(image, blurred(PORTRAIT, lambda d: d.ellipse([360, 220, 920, 860], fill=rim), 60))
    draw = ImageDraw.Draw(image)
    figure = (12, 10, 20)
    # Shoulders and neck.
    draw.polygon([(200, 1280), (330, 980), (560, 900), (720, 900), (950, 980), (1080, 1280)], fill=figure)
    draw.rectangle([570, 760, 710, 940], fill=figure)
    # Head.
    draw.ellipse([450, 380, 830, 820], fill=figure)
    if hat:
        draw.ellipse([330, 440, 950, 520], fill=figure)
        draw.rounded_rectangle([470, 280, 810, 480], 40, fill=figure)
    else:
        # Bob haircut.
        draw.chord([420, 350, 860, 820], 180, 360, fill=figure)
        draw.rectangle([420, 585, 470, 780], fill=figure)
        draw.rectangle([810, 585, 860, 780], fill=figure)
    return image


def blurred(size, shape, radius):
    layer = Image.new("RGB", size, (0, 0, 0))
    shape(ImageDraw.Draw(layer))
    return layer.filter(ImageFilter.GaussianBlur(radius))


def main():
    output = Path(sys.argv[1])
    output.mkdir(parents=True, exist_ok=True)
    pictures = {
        "fondo-oficina.png": office(),
        "fondo-club.png": club(),
        "retrato-lola-neutral.png": portrait((18, 60, 72), (6, 16, 24), (220, 60, 150), False),
        "retrato-damaso-neutral.png": portrait((70, 28, 12), (16, 6, 6), (230, 150, 40), True),
    }
    for name, picture in pictures.items():
        picture.save(output / name, format="PNG", optimize=True)
        print(name)


if __name__ == "__main__":
    main()
