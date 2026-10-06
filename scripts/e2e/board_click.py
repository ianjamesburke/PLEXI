#!/usr/bin/env python3
"""Locate a UCI move on a Plexi chess-board screenshot.

Prints four window-relative pixels: from_x from_y to_x to_y.

apps/chess paints light squares #a6adc8, the cursor #89b4fa, and a
selected square #f9e2af. The host framebuffer shifts the light square
toward #cdd6f4, so matching uses those rendered colors. Text of the
same light color is dropped because it is not a square blob.
"""
from __future__ import annotations

import sys

from PIL import Image

LIGHT = (205, 214, 244)
CURSOR = (137, 180, 250)
SELECTED = (249, 226, 175)
TOL = 24


def close(pixel: tuple, target: tuple) -> bool:
    return all(abs(int(pixel[i]) - target[i]) <= TOL for i in range(3))


def square_pixel(pixel: tuple) -> bool:
    return close(pixel, LIGHT) or close(pixel, CURSOR) or close(pixel, SELECTED)


def components(image: Image.Image) -> list[tuple[int, int, int, int, int]]:
    """Bounding boxes of square-colored blobs: (x0, y0, x1, y1, count)."""
    rgb = image.convert("RGB")
    width, height = rgb.size
    pixels = rgb.load()
    seen = [[False] * width for _ in range(height)]
    found: list[tuple[int, int, int, int, int]] = []
    for y in range(height):
        for x in range(width):
            if seen[y][x] or not square_pixel(pixels[x, y]):
                continue
            stack = [(x, y)]
            seen[y][x] = True
            min_x = max_x = x
            min_y = max_y = y
            count = 0
            while stack:
                cx, cy = stack.pop()
                count += 1
                if cx < min_x:
                    min_x = cx
                if cy < min_y:
                    min_y = cy
                if cx > max_x:
                    max_x = cx
                if cy > max_y:
                    max_y = cy
                for nx, ny in ((cx + 1, cy), (cx - 1, cy), (cx, cy + 1), (cx, cy - 1)):
                    if nx < 0 or ny < 0 or nx >= width or ny >= height:
                        continue
                    if seen[ny][nx] or not square_pixel(pixels[nx, ny]):
                        continue
                    seen[ny][nx] = True
                    stack.append((nx, ny))
            found.append((min_x, min_y, max_x + 1, max_y + 1, count))
    return found


def median(values: list[float]) -> float:
    ordered = sorted(values)
    mid = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[mid]
    return (ordered[mid - 1] + ordered[mid]) / 2.0


def square_blobs(image: Image.Image) -> list[tuple[float, float, float]]:
    """Centers and widths of blobs that are board squares, not glyphs."""
    blobs = components(image)
    sized = []
    for x0, y0, x1, y1, count in blobs:
        width = x1 - x0
        height = y1 - y0
        if width < 8 or height < 8 or count < 40:
            continue
        aspect = width / height
        if aspect < 0.65 or aspect > 1.45:
            continue
        sized.append((width, height, count, x0, y0, x1, y1))
    if not sized:
        raise SystemExit("board not found in screenshot (no square blobs)")
    typical = median([float(item[0]) for item in sized])
    kept: list[tuple[float, float, float]] = []
    for width, _height, _count, x0, y0, x1, y1 in sized:
        if width < typical * 0.55 or width > typical * 1.6:
            continue
        kept.append(((x0 + x1) / 2.0, (y0 + y1) / 2.0, width))
    if len(kept) < 8:
        raise SystemExit(
            f"board not found in screenshot ({len(kept)} squares, typical {typical:.0f}px)"
        )
    return kept


def clusters(values: list[float], gap: float) -> list[float]:
    ordered = sorted(values)
    groups: list[list[float]] = [[ordered[0]]]
    for value in ordered[1:]:
        if value - groups[-1][-1] <= gap:
            groups[-1].append(value)
        else:
            groups.append([value])
    return [sum(group) / len(group) for group in groups]


def complete_axis(centers: list[float], cell: float) -> list[float]:
    """Eight file or rank centers. Insert a missing edge from the cell size."""
    axis = clusters(centers, cell * 0.45)
    if len(axis) > 8:
        raise SystemExit(f"board axis has {len(axis)} clusters")
    while len(axis) < 8:
        gaps = [axis[i + 1] - axis[i] for i in range(len(axis) - 1)]
        if gaps and max(gaps) > cell * 1.5:
            index = gaps.index(max(gaps))
            axis.insert(index + 1, (axis[index] + axis[index + 1]) / 2.0)
            continue
        # A missing edge sits one cell outside the nearer end.
        if axis[0] - cell >= 0:
            axis.insert(0, axis[0] - cell)
        else:
            axis.append(axis[-1] + cell)
    return axis


def board_axes(image: Image.Image) -> tuple[list[float], list[float]]:
    blobs = square_blobs(image)
    cell = median([item[2] for item in blobs])
    files = complete_axis([item[0] for item in blobs], cell)
    ranks = complete_axis([item[1] for item in blobs], cell)
    if len(files) != 8 or len(ranks) != 8:
        raise SystemExit(f"board grid is {len(files)}x{len(ranks)}")
    return files, ranks


def parse_square(text: str) -> tuple[int, int]:
    file_idx = ord(text[0]) - ord("a")
    rank_idx = int(text[1]) - 1
    if not (0 <= file_idx <= 7 and 0 <= rank_idx <= 7):
        raise SystemExit(f"bad square {text}")
    return file_idx, rank_idx


def center(files: list[float], ranks: list[float], file_idx: int, rank_idx: int) -> tuple[int, int]:
    # ranks[0] is the top of the board (chess rank 8). Rank 1 is the bottom.
    return int(files[file_idx]), int(ranks[7 - rank_idx])


def main() -> None:
    image = Image.open(sys.argv[1])
    uci = sys.argv[2].strip().lower()
    if len(uci) < 4:
        raise SystemExit(f"bad uci {uci}")
    files, ranks = board_axes(image)
    src = center(files, ranks, *parse_square(uci[0:2]))
    dst = center(files, ranks, *parse_square(uci[2:4]))
    print(f"{src[0]} {src[1]} {dst[0]} {dst[1]}")


if __name__ == "__main__":
    main()
