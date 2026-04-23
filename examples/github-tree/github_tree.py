#!/usr/bin/env python3
"""GitHub Tree — browse the current git repo's tree via the `gh` CLI.

The app inherits the launching pane's cwd. On startup it:
  1. Confirms `gh` is authenticated.
  2. Resolves the enclosing git repo and its owner/repo slug.
  3. Fetches the repo tree via `gh api repos/<slug>/git/trees/HEAD?recursive=1`.

Auth is delegated entirely to the `gh` CLI — no secrets broker, no net.http.
Run `gh auth login` in any terminal to set up credentials; they live at
`~/.config/gh/hosts.yml`, which is reachable because the subprocess env
whitelist includes HOME + PATH.

Keys:
  h / l / left / right — move between columns
  j / k / up / down    — move within a column
  r                    — refetch the tree
"""
from __future__ import annotations

import re
import shutil
import subprocess
import threading
from typing import Optional

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    Column, Card, Header, Spacer, Footer, Label, KeyRow,
    ACCENT, BG, FG, MUTED, HIGHLIGHT, RED, YELLOW,
    SPACE_MD, SPACE_XL,
    TEXT_CAPTION, TEXT_BODY,
)

MODE_LOADING  = "loading"
MODE_NO_AUTH  = "no_auth"
MODE_NO_REPO  = "no_repo"
MODE_READY    = "ready"
MODE_ERROR    = "error"

SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]

# Remote URL parsers: both git@github.com:owner/repo(.git) and
# https://github.com/owner/repo(.git) forms.
_RE_SSH_REMOTE  = re.compile(r"^git@github\.com:([^/]+)/(.+?)(?:\.git)?$")
_RE_HTTP_REMOTE = re.compile(r"^https?://github\.com/([^/]+)/(.+?)(?:\.git)?/?$")


def _run(cmd: list[str], cwd: Optional[str] = None,
         timeout: float = 10.0) -> tuple[int, str, str]:
    """Run `cmd` and return (returncode, stdout, stderr). Never raises on a
    non-zero exit — caller inspects the return code. Failures to spawn (missing
    binary, timeout) become synthetic non-zero return codes so the caller sees
    a uniform shape."""
    try:
        proc = subprocess.run(
            cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout,
        )
        return proc.returncode, proc.stdout, proc.stderr
    except FileNotFoundError as e:
        return 127, "", f"command not found: {cmd[0]} ({e})"
    except subprocess.TimeoutExpired as e:
        return 124, "", f"timeout running {cmd[0]}: {e}"
    except Exception as e:
        return 1, "", f"error running {cmd[0]}: {e}"


def _parse_slug(remote_url: str) -> Optional[tuple[str, str]]:
    url = remote_url.strip()
    m = _RE_SSH_REMOTE.match(url)
    if m:
        return m.group(1), m.group(2)
    m = _RE_HTTP_REMOTE.match(url)
    if m:
        return m.group(1), m.group(2)
    return None


class GitHubTreeApp(App):

    # ── Lifecycle ─────────────────────────────────────────────────────────────

    def on_init(self, _ctx: RenderContext) -> None:
        self._mode: str = MODE_LOADING
        self._loading_msg: str = "Checking gh auth…"
        self._spinner_frame = 0

        self._repo_root: Optional[str] = None
        self._slug: Optional[str] = None           # "owner/repo"
        self._tree: list[str] = []                 # repo-relative paths (blobs only)
        self._error: str = ""

        # Grid selection — index into self._tree.
        self._sel: int = 0

        # Last-computed column grid, stashed so the key handler can step by
        # columns (sel += rows) without recomputing.
        self._grid_cols: int = 1
        self._grid_rows: int = 1

        self.emit.status_summary("GitHub Tree — loading")
        self.emit.info("github-tree: starting")

        threading.Thread(target=self._bootstrap, daemon=True).start()

    # ── Bootstrap (auth → repo detect → tree fetch) ──────────────────────────

    def _bootstrap(self) -> None:
        try:
            # 1. gh present?
            if shutil.which("gh") is None:
                self._fail_auth("The gh CLI isn't on PATH. Install it, then run "
                                "`gh auth login`.")
                return

            # 2. gh authenticated?
            self._loading_msg = "Checking gh auth…"
            self.emit.schedule_render(after_ms=16)
            rc, _, _ = _run(["gh", "auth", "status"])
            if rc != 0:
                self._fail_auth("Run `gh auth login` in any terminal, then reopen.")
                return

            # 3. In a git repo?
            self._loading_msg = "Locating git repo…"
            self.emit.schedule_render(after_ms=16)
            rc, out, _ = _run(["git", "rev-parse", "--show-toplevel"])
            if rc != 0 or not out.strip():
                self._mode = MODE_NO_REPO
                self.emit.status_summary("GitHub Tree — no repo")
                self.emit.schedule_render(after_ms=16)
                return
            self._repo_root = out.strip()

            # 4. Parse owner/repo from origin remote.
            rc, out, err = _run(["git", "remote", "get-url", "origin"],
                                cwd=self._repo_root)
            if rc != 0 or not out.strip():
                self._fail_error(f"No 'origin' remote: {err.strip() or 'not set'}")
                return
            parsed = _parse_slug(out.strip())
            if not parsed:
                self._fail_error(f"Origin remote is not a GitHub URL:\n{out.strip()}")
                return
            owner, repo = parsed
            self._slug = f"{owner}/{repo}"
            self.emit.status_summary(self._slug)

            # 5. Fetch the tree via gh api.
            self._fetch_tree()

        except Exception as e:
            self._fail_error(f"bootstrap failed: {e}")

    def _fetch_tree(self) -> None:
        if self._slug is None:
            self._fail_error("no repo slug resolved")
            return
        self._mode = MODE_LOADING
        self._loading_msg = f"Fetching tree for {self._slug}…"
        self.emit.schedule_render(after_ms=16)

        rc, out, err = _run(
            ["gh", "api",
             f"repos/{self._slug}/git/trees/HEAD?recursive=1",
             "--jq", ".tree[] | select(.type==\"blob\") | .path"],
            cwd=self._repo_root,
            timeout=30.0,
        )
        if rc != 0:
            self._fail_error(
                f"gh api failed: {err.strip() or out.strip() or 'unknown error'}"
            )
            return

        paths = sorted(line for line in out.splitlines() if line.strip())
        self._tree = paths
        self._sel = 0
        self._mode = MODE_READY
        self.emit.info(f"github-tree: {len(paths)} blobs in {self._slug}")
        self.emit.schedule_render(after_ms=16)

    # Transition helpers — keep `_mode` moves centralised so every render sees
    # a consistent (mode, error) snapshot.
    def _fail_auth(self, message: str) -> None:
        self._error = message
        self._mode = MODE_NO_AUTH
        self.emit.status_summary("GitHub Tree — not authed")
        self.emit.schedule_render(after_ms=16)

    def _fail_error(self, message: str) -> None:
        self._error = message
        self._mode = MODE_ERROR
        self.emit.warn(f"github-tree: {message}")
        self.emit.status_summary("GitHub Tree — error")
        self.emit.schedule_render(after_ms=16)

    # ── Input ─────────────────────────────────────────────────────────────────

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        k = key.lower()
        if self._mode == MODE_READY:
            self._handle_ready_key(k)
        elif self._mode in (MODE_ERROR, MODE_NO_AUTH, MODE_NO_REPO):
            if k in ("r", "return"):
                threading.Thread(target=self._bootstrap, daemon=True).start()

    def _handle_ready_key(self, k: str) -> None:
        if not self._tree:
            return
        rows = max(1, self._grid_rows)
        total = len(self._tree)

        if k in ("j", "down"):
            nxt = self._sel + 1
            if nxt < total and (nxt % rows) != 0:
                self._sel = nxt
        elif k in ("k", "up"):
            if (self._sel % rows) != 0:
                self._sel -= 1
        elif k in ("l", "right"):
            nxt = self._sel + rows
            if nxt < total:
                self._sel = nxt
        elif k in ("h", "left"):
            nxt = self._sel - rows
            if nxt >= 0:
                self._sel = nxt
        elif k == "r":
            threading.Thread(target=self._fetch_tree, daemon=True).start()
        self.emit.schedule_render(after_ms=16)

    # ── Render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        if self._mode == MODE_LOADING:
            self._spinner_frame = (self._spinner_frame + 1) % len(SPINNER)
            ctx.emit.schedule_render(after_ms=80)
            self._render_loading(ctx)
        elif self._mode == MODE_NO_AUTH:
            self._render_empty_state(
                ctx,
                title="gh CLI not authenticated",
                body=self._error or "Run `gh auth login` in any terminal, then reopen.",
                accent=YELLOW,
            )
        elif self._mode == MODE_NO_REPO:
            self._render_empty_state(
                ctx,
                title="Not in a git repository",
                body="Launch from a repo terminal (the app inherits its cwd).",
                accent=YELLOW,
            )
        elif self._mode == MODE_ERROR:
            self._render_empty_state(
                ctx,
                title="Something went wrong",
                body=self._error or "unknown error",
                accent=RED,
            )
        elif self._mode == MODE_READY:
            self._render_ready(ctx)

    # ── Chrome for loading/empty/error states ────────────────────────────────
    def _render_loading(self, ctx: RenderContext) -> None:
        ctx.render(Column([
            Header(title="GitHub Tree", subtitle=self._slug or "Resolving…"),
            Card([
                Label(f"{SPINNER[self._spinner_frame]}  {self._loading_msg}",
                      tone="body", color=ACCENT),
            ]),
            Spacer(grow=True),
            Footer("Auth via `gh` CLI · tree fetched via `gh api`"),
        ]))

    def _render_empty_state(self, ctx: RenderContext, *,
                            title: str, body: str, accent: str) -> None:
        ctx.render(Column([
            Header(title="GitHub Tree", subtitle=title, accent=accent),
            Card([
                Label(body, tone="body"),
            ]),
            Card([
                KeyRow("r", "Retry"),
            ]),
            Spacer(grow=True),
            Footer("Auth via `gh` CLI · inherits the launching pane's cwd"),
        ]))

    # Ready state: Header band on top, Footer band on bottom, horizontal grid
    # of file paths in between. Chrome uses SDK v2; the grid uses primitives
    # because SDK v2 has no multi-column list component.
    def _render_ready(self, ctx: RenderContext) -> None:
        ctx.clear(BG)

        header = Header(title="GitHub Tree", subtitle=self._slug or "—")
        header_h = header.measure(ctx.w - 2 * SPACE_XL)
        header.render(
            ctx,
            SPACE_XL, SPACE_XL,
            ctx.w - 2 * SPACE_XL, header_h,
        )

        footer_text = (
            f"{len(self._tree)} files · "
            "h/l columns · j/k within column · r refresh"
        )
        footer = Footer(footer_text)
        footer_h = footer.measure(ctx.w - 2 * SPACE_XL)
        footer_y = ctx.h - SPACE_XL - footer_h
        footer.render(
            ctx,
            SPACE_XL, footer_y,
            ctx.w - 2 * SPACE_XL, footer_h,
        )

        grid_x = SPACE_XL
        grid_y = SPACE_XL + header_h + SPACE_MD
        grid_w = ctx.w - 2 * SPACE_XL
        grid_h = footer_y - grid_y - SPACE_MD
        if grid_h <= 0 or grid_w <= 0:
            return

        self._draw_grid(ctx, grid_x, grid_y, grid_w, grid_h)

    # ── Horizontal column grid ────────────────────────────────────────────────
    # Items flow column-major: column 0 fills top-to-bottom, then column 1, etc.
    # h/l moves between columns (sel ± rows), j/k moves within a column.
    # When the selection's column is off-screen we shift the visible window so
    # the selection is always visible (rightmost if scrolling right).

    ROW_H       = 20.0
    COL_GAP     = SPACE_MD
    COL_MIN_W   = 160.0   # anything narrower and paths truncate brutally
    COL_MAX_W   = 320.0

    def _draw_grid(self, ctx: RenderContext, x: float, y: float,
                   w: float, h: float) -> None:
        if not self._tree:
            ctx.text(x, y, "(empty tree)", size=TEXT_BODY, color=MUTED)
            self._grid_rows = 1
            self._grid_cols = 1
            return

        rows = max(1, int(h // self.ROW_H))
        max_cols_by_width = max(1, int(
            (w + self.COL_GAP) // (self.COL_MIN_W + self.COL_GAP)
        ))
        cols_needed = (len(self._tree) + rows - 1) // rows
        cols = min(max_cols_by_width, max(1, cols_needed))

        col_w = (w - self.COL_GAP * (cols - 1)) / cols
        if col_w > self.COL_MAX_W:
            col_w = self.COL_MAX_W

        self._grid_rows = rows
        self._grid_cols = cols

        # Scroll window: if the selection's column has scrolled off the
        # right edge, shift so it's the rightmost visible column.
        sel_col = self._sel // rows
        col_offset = 0
        if sel_col >= cols:
            col_offset = sel_col - cols + 1

        for visible_col in range(cols):
            src_col = visible_col + col_offset
            for row in range(rows):
                idx = src_col * rows + row
                if idx >= len(self._tree):
                    break
                cx = x + visible_col * (col_w + self.COL_GAP)
                cy = y + row * self.ROW_H
                path = self._tree[idx]
                selected = idx == self._sel

                if selected:
                    ctx.rect(cx - 4.0, cy, col_w + 4.0, self.ROW_H,
                             fill=HIGHLIGHT, radius=4.0)
                color = ACCENT if selected else FG
                display = _fit_path(path, col_w)
                ctx.text(cx, cy + self.ROW_H / 2.0, display,
                         size=TEXT_CAPTION, color=color,
                         monospace=True, align="left_center")


def _fit_path(path: str, avail_px: float) -> str:
    """Truncate from the LEFT so the filename (tail) stays visible. Mono glyph
    width ≈ 0.60 * font_size; we render at TEXT_CAPTION."""
    char_w = TEXT_CAPTION * 0.60
    max_chars = max(4, int(avail_px / char_w))
    if len(path) <= max_chars:
        return path
    return "…" + path[-(max_chars - 1):]


if __name__ == "__main__":
    GitHubTreeApp().run()
