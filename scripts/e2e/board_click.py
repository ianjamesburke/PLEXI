#!/usr/bin/env python3
"""Locate a UCI move on a Plexi chess-board screenshot.

Prints four window-relative pixels: from_x from_y to_x to_y.
The board is the checker of #a6adc8 / #45475a squares drawn by apps/chess.
"""
from __future__ import annotations

import sys

from PIL import Image

LIGHT = (166, 173, 200)
DARK = (69, 71, 90)
TOL = 36


def close(pixel: tuple, target: tuple) -> bool:
    return all(abs(int(pixel[i]) - target[i]) <= TOL for i in range(3))


def square_mask(image: Image.Image) -> list[list[bool]]:
    rgb = image.convert("RGB")
    width, height = rgb.size
    pixels = rgb.load()
    return [
        [close(pixels[x, y], LIGHT) or close(pixels[x, y], DARK) for x in range(width)]
        for y in range(height)
    ]


def board_box(mask: list[list[bool]]) -> tuple[int, int, int, int]:
    height = len(mask)
    width = len(mask[0]) if height else 0
    min_x, min_y, max_x, max_y = width, height, 0, 0
    count = 0
    for y in range(height):
        row = mask[y]
        for x in range(width):
            if row[x]:
                count += 1
                if x < min_x:
                    min_x = x
                if y < min_y:
                    min_y = y
                if x > max_x:
                    max_x = x
                if y > max_y:
                    max_y = y
    if count < 64:
        raise SystemExit("board not found in screenshot")
    return min_x, min_y, max_x + 1, max_y + 1


def parse_square(text: str) -> tuple[int, int]:
    file_idx = ord(text[0]) - ord("a")
    rank_idx = int(text[1]) - 1
    if not (0 <= file_idx <= 7 and 0 <= rank_idx <= 7):
        raise SystemExit(f"bad square {text}")
    return file_idx, rank_idx


def center(box: tuple[int, int, int, int], file_idx: int, rank_idx: int) -> tuple[int, int]:
    x0, y0, x1, y1 = box
    cell_w = (x1 - x0) / 8.0
    cell_h = (y1 - y0) / 8.0
    # rank 0 (chess rank 1) is the bottom row.
    row_from_top = 7 - rank_idx
    return (
        int(x0 + (file_idx + 0.5) * cell_w),
        int(y0 + (row_from_top + 0.5) * cell_h),
    )


def main() -> None:
    image = Image.open(sys.argv[1])
    uci = sys.argv[2].strip().lower()
    if len(uci) < 4:
        raise SystemExit(f"bad uci {uci}")
    box = board_box(square_mask(image))
    src = center(box, *parse_square(uci[0:2]))
    dst = center(box, *parse_square(uci[2:4]))
    print(f"{src[0]} {src[1]} {dst[0]} {dst[1]}")


if __name__ == "__main__":
    main()
