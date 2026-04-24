#!/usr/bin/env python3
from __future__ import annotations

"""Commit Graph — subway-style git history viewer for Plexi.

One pane, no `gh` dependency. Reads the local git repo directly.
"""

import os
import threading
import time
from typing import Optional

from plexi_sdk import App, RenderContext, dim
from plexi_sdk import BG, SURFACE, HIGHLIGHT, ACCENT, MUTED, FG, RED, GREEN, YELLOW
from plexi_sdk.ui import (
    Column, Card, Header, Spacer, Footer, FooterKeys, Label, KeyRow,
    TEXT_CAPTION, TEXT_BODY, TEXT_HINT, TEXT_HEADING,
    SPACE_MD, SPACE_LG, SPACE_XL,
    RADIUS_SM, RADIUS_MD,
    badge,
    _wrap_to_width,
)

import git_log as gl

# ── Layout constants ──────────────────────────────────────────────────────────
PAD_X       = 24.0
LANE_W      = 28.0
ROW_H       = 28.0
NODE_R      = 5.0
HEAD_RING   = 8.0
LEGEND_H    = 22.0   # height of the legend row
LEGEND_PAD  = 8.0    # vertical padding around the legend strip

# ── App modes ─────────────────────────────────────────────────────────────────
MODE_LOADING = "loading"
MODE_NO_REPO = "no_repo"
MODE_NO_GIT  = "no_git"
MODE_READY   = "ready"
MODE_ERROR   = "error"

SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]

# ── Stale poll interval ───────────────────────────────────────────────────────
POLL_INTERVAL_S = 3.0


class CommitGraphApp(App):

    # ── Lifecycle ─────────────────────────────────────────────────────────────

    def on_init(self, ctx: RenderContext) -> None:
        self._mode: str = MODE_LOADING
        self._loading_msg: str = "Locating git repo…"
        self._spinner_start: float = time.monotonic()
        self._error: str = ""

        self._repo_root: Optional[str] = None
        self._repo_name: str = ""
        self._head_sha: Optional[str] = None
        self._head_branch: Optional[str] = None
        self._origin_url: Optional[str] = None

        # Week viewport: 0 = current week, 1 = last week, etc.
        self._week_offset: int = 0

        # Parsed graph data
        self._commits: list[dict] = []
        self._refs: list[dict] = []
        self._edges: list[tuple[str, str]] = []
        self._stats_unavailable: bool = False

        # Selection (index into self._commits, newest-first order)
        self._sel: int = 0

        # Tooltip / help overlay
        # Note: no tooltip state — selection drives the always-visible details
        # panel on the right side. No toggle, no Enter/Esc dance.
        self._show_help: bool = False

        # Hit-test list: [(hash, cx, cy, r)] rebuilt each render
        self._hit_nodes: list[tuple[str, float, float, float]] = []

        # Stale-check state
        self._last_head_sha: Optional[str] = None
        self._last_poll_time: float = 0.0

        self.emit.status_summary("Commit Graph — loading")
        self.emit.schedule_render(after_ms=16)

        threading.Thread(target=self._bootstrap, daemon=True).start()

    # ── Bootstrap ─────────────────────────────────────────────────────────────

    def _bootstrap(self) -> None:
        try:
            cwd = self.workspace_root or os.getcwd()
            self.emit.debug(
                f"bootstrap: workspace_root={self.workspace_root!r} "
                f"os.getcwd={os.getcwd()!r} chosen_cwd={cwd!r}"
            )
            root = gl.find_repo_root(cwd)
            self.emit.debug(f"bootstrap: find_repo_root({cwd!r}) -> {root!r}")
            if root is None:
                self._set_mode(MODE_NO_REPO)
                return
            self._repo_root = root
            self._repo_name = os.path.basename(root)
            self._head_branch = gl.get_head_branch(root)
            self._origin_url = gl.get_origin_remote(root)
            self.emit.debug(
                f"bootstrap: repo_name={self._repo_name!r} "
                f"head_branch={self._head_branch!r} "
                f"origin={self._origin_url!r}"
            )
            self._fetch_graph()
        except Exception as e:
            self._error = f"bootstrap failed: {e}"
            self._set_mode(MODE_ERROR)

    def _fetch_graph(self) -> None:
        """Fetch refs + commits + numstats for the current week viewport."""
        if self._repo_root is None:
            return

        self._set_mode(MODE_LOADING, "Fetching commits…")

        import time as _time
        now_ts = int(_time.time())
        end_ts = now_ts - self._week_offset * 7 * 86400
        start_ts = end_ts - 7 * 86400

        try:
            self.emit.debug(
                f"fetch_graph: repo_root={self._repo_root!r} "
                f"week_offset={self._week_offset} "
                f"window=[{start_ts}..{end_ts}] "
                f"now={now_ts}"
            )
            # Refs + commits fetched; numstats can run in parallel but we keep
            # it simple (GIL-safe subprocess calls) — sequential is fine.
            refs = gl.fetch_refs(self._repo_root, now_ts)
            self.emit.debug(f"fetch_graph: fetched {len(refs)} refs")
            commits = gl.fetch_commits(self._repo_root, start_ts, end_ts)
            self.emit.debug(f"fetch_graph: fetched {len(commits)} commits in window")

            # Assign lanes and colours
            gl.assign_lanes(commits, refs)

            # Fetch numstats (capped at 2000 commits)
            stats = gl.fetch_numstats(
                self._repo_root, start_ts, end_ts, len(commits)
            )
            self._stats_unavailable = (
                len(commits) > gl._MAX_COMMITS_FOR_STATS
            )
            for c in commits:
                if c["hash"] in stats:
                    c["added"], c["removed"] = stats[c["hash"]]

            edges = gl.build_edges(commits)

            self._refs = refs
            self._commits = commits
            self._edges = edges
            self._sel = min(self._sel, max(0, len(commits) - 1))
            self._head_sha = gl.get_head_sha(self._repo_root)
            self._last_head_sha = self._head_sha
            self._last_poll_time = time.monotonic()

            self._set_mode(MODE_READY)
        except Exception as e:
            self._error = f"fetch failed: {e}"
            self._set_mode(MODE_ERROR)

    def _set_mode(self, mode: str, msg: str = "") -> None:
        self._mode = mode
        if msg:
            self._loading_msg = msg
        if mode == MODE_NO_REPO:
            self.emit.status_summary("Commit Graph — not a repo")
        elif mode == MODE_NO_GIT:
            self.emit.status_summary("Commit Graph — git not found")
        elif mode == MODE_ERROR:
            self.emit.warn(f"commit-graph: {self._error}")
            self.emit.status_summary("Commit Graph — error")
        elif mode == MODE_READY:
            n = len(self._commits)
            self.emit.status_summary(f"Commit Graph — {self._repo_name} · {n} commits")
        self.emit.schedule_render(after_ms=16)

    # ── Polling (lightweight — no file-watcher dependency) ────────────────────

    def _maybe_poll(self) -> None:
        """Check if HEAD has changed; re-fetch if so. Called from on_render."""
        if self._repo_root is None or self._mode != MODE_READY:
            return
        now = time.monotonic()
        if now - self._last_poll_time < POLL_INTERVAL_S:
            return
        self._last_poll_time = now
        current_sha = gl.get_head_sha(self._repo_root)
        if current_sha and current_sha != self._last_head_sha:
            threading.Thread(target=self._fetch_graph, daemon=True).start()

    # ── Input ─────────────────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:  # noqa: C901
        if self._mode in (MODE_NO_REPO, MODE_NO_GIT, MODE_ERROR):
            if key == "r":
                threading.Thread(target=self._bootstrap, daemon=True).start()
            return

        if self._mode == MODE_LOADING:
            return

        # Help toggle
        if key == "?":
            self._show_help = not self._show_help
            self.emit.schedule_render(after_ms=0)
            return

        if self._show_help:
            # Any key dismisses help
            self._show_help = False
            self.emit.schedule_render(after_ms=0)
            return

        # Esc clears the selection. Detail panel falls back to placeholder.
        # No tooltip toggle — selection drives the always-visible panel.
        if key == "escape":
            self._sel = -1
            self.emit.schedule_render(after_ms=0)
            return

        n = len(self._commits)

        if key in ("[",):
            self._week_offset += 1
            self._sel = 0
            threading.Thread(target=self._fetch_graph, daemon=True).start()
        elif key in ("]",):
            if self._week_offset > 0:
                self._week_offset -= 1
                self._sel = 0
                threading.Thread(target=self._fetch_graph, daemon=True).start()
        elif key == "t":
            if self._week_offset != 0:
                self._week_offset = 0
                self._sel = 0
                threading.Thread(target=self._fetch_graph, daemon=True).start()
        elif key in ("j", "down") and n > 0:
            self._sel = min(self._sel + 1, n - 1)
            self.emit.schedule_render(after_ms=0)
        elif key in ("k", "up") and n > 0:
            self._sel = max(self._sel - 1, 0)
            self.emit.schedule_render(after_ms=0)
        elif key in ("h", "left") and n > 0:
            cur_lane = self._commits[self._sel]["lane"] if n > 0 else 0
            target_lane = cur_lane - 1
            if target_lane >= 0:
                best = self._nearest_in_lane(target_lane)
                if best is not None:
                    self._sel = best
                    self.emit.schedule_render(after_ms=0)
        elif key in ("l", "right") and n > 0:
            cur_lane = self._commits[self._sel]["lane"] if n > 0 else 0
            target_lane = cur_lane + 1
            best = self._nearest_in_lane(target_lane)
            if best is not None:
                self._sel = best
                self.emit.schedule_render(after_ms=0)
        elif key == "g":
            self._sel = 0
            self.emit.schedule_render(after_ms=0)
        elif key == "G":
            self._sel = max(0, n - 1)
            self.emit.schedule_render(after_ms=0)
        elif key == "c":
            # IMPL-NOTE: SDK has no clipboard emit — log + status line instead.
            if n > 0:
                full_hash = self._commits[self._sel]["hash"]
                self.emit.info(f"commit-graph: copy hash {full_hash}")
                self.emit.status_summary(f"Hash: {full_hash}")
                self.emit.schedule_render(after_ms=0)
        elif key == "o":
            self._open_on_github()
        elif key == "r":
            threading.Thread(target=self._fetch_graph, daemon=True).start()

    def _nearest_in_lane(self, lane: int) -> Optional[int]:
        """Return index of commit in `lane` nearest in row to current sel."""
        if not self._commits:
            return None
        cur_idx = self._sel
        candidates = [
            i for i, c in enumerate(self._commits) if c["lane"] == lane
        ]
        if not candidates:
            return None
        return min(candidates, key=lambda i: abs(i - cur_idx))

    def _open_on_github(self) -> None:
        if not self._commits or self._origin_url is None:
            return
        import re
        url = self._origin_url
        m = re.search(r"github\.com[:/]([^/]+/[^/]+?)(?:\.git)?$", url)
        if not m:
            return
        slug = m.group(1)
        commit_hash = self._commits[self._sel]["hash"]
        gh_url = f"https://github.com/{slug}/commit/{commit_hash}"
        # IMPL-NOTE: SDK has no open-url emit — log the URL as best effort.
        self.emit.info(f"commit-graph: open {gh_url}")
        self.emit.status_summary(f"URL: {gh_url}")
        self.emit.schedule_render(after_ms=16)

    def on_click(self, ctx: RenderContext, x: float, y: float, button: str) -> None:
        if self._mode != MODE_READY:
            return
        for i, (h, cx, cy, r) in enumerate(self._hit_nodes):
            if (x - cx) ** 2 + (y - cy) ** 2 <= (r + 4) ** 2:
                for j, c in enumerate(self._commits):
                    if c["hash"] == h:
                        self._sel = j
                        self._show_help = False
                        # Snappy click: 0ms = repaint as soon as possible.
                        # 16ms scheduled the next frame ~16-32ms out, which
                        # felt sluggish. The selection is purely local
                        # state — no I/O — so immediate repaint is safe.
                        self.emit.schedule_render(after_ms=0)
                        return
                break

    # ── Render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:  # noqa: C901
        self._maybe_poll()

        if self._mode == MODE_LOADING:
            self._render_loading(ctx)
            # Keep advancing the spinner at a calm 10 fps regardless of what
            # else causes renders. Frame index is derived from wall-clock
            # elapsed time, not incremented per-render, so event-driven
            # redraws don't speed it up.
            self.emit.schedule_render(after_ms=100)
            return

        if self._mode == MODE_NO_REPO:
            self._render_error_state(
                ctx,
                title="Not a git repository",
                lines=[
                    "Launch from a repo terminal.",
                    "Press r to retry.",
                ],
            )
            return

        if self._mode == MODE_NO_GIT:
            self._render_error_state(
                ctx,
                title="`git` isn't on PATH",
                lines=["Install git, then press r to retry."],
            )
            return

        if self._mode == MODE_ERROR:
            self._render_error_state(
                ctx,
                title="Something went wrong",
                lines=[self._error or "unknown error", "Press r to retry."],
            )
            return

        if self._mode == MODE_READY:
            self._render_ready(ctx)

    def _render_loading(self, ctx: RenderContext) -> None:
        # Spinner frame derived from wall-clock time at 8 fps — stays calm
        # even if on_render is called more frequently than the 100 ms tick.
        idx = int((time.monotonic() - self._spinner_start) * 8) % len(SPINNER)
        ctx.render(Column([
            Header(title="Commit Graph", subtitle=self._repo_name or "Resolving…"),
            Card([
                Label(
                    f"{SPINNER[idx]}  {self._loading_msg}",
                    tone="body",
                    color=ACCENT,
                ),
            ]),
            Spacer(grow=True),
            Footer("Subway-style git history · local git only"),
        ]))

    def _render_error_state(self, ctx: RenderContext, *,
                            title: str, lines: list[str]) -> None:
        ctx.render(Column([
            Header(title="Commit Graph", subtitle=title, accent=RED),
            Card([Label(l, tone="body") for l in lines]),
            Card([KeyRow("r", "Retry")]),
            Spacer(grow=True),
            Footer("[ ] week  t today  r refresh  ? help"),
        ]))

    def _render_ready(self, ctx: RenderContext) -> None:  # noqa: C901
        ctx.clear(BG)

        import time as _time
        now_ts = int(_time.time())
        end_ts = now_ts - self._week_offset * 7 * 86400
        start_ts = end_ts - 7 * 86400

        # ── Header ────────────────────────────────────────────────────────────
        week_str = self._format_week(start_ts, end_ts)
        n = len(self._commits)
        subtitle = f"{self._repo_name} · {week_str} · {n} commit{'s' if n != 1 else ''}"
        if self._week_offset > 0:
            subtitle += f" (–{self._week_offset}w)"

        # Measure and render Header manually so we can position the rest below it
        header = Header(title="Commit Graph", subtitle=subtitle)
        hdr_h = header.measure(ctx.w - 2 * SPACE_XL)
        header.render(ctx, SPACE_XL, SPACE_XL, ctx.w - 2 * SPACE_XL, hdr_h)
        y_cursor = SPACE_XL + hdr_h + SPACE_MD

        # ── Footer ────────────────────────────────────────────────────────────
        footer = FooterKeys([
            (["[", "]"], "week"),
            ("t", "today"),
            (["j", "k"], "commit"),
            (["h", "l"], "lane"),
            (["g", "G"], "ends"),
            ("c", "copy"),
            ("o", "open"),
            ("r", "refresh"),
            ("?", "help"),
        ])
        ftr_h = footer.measure(ctx.w - 2 * SPACE_XL)
        footer_y = ctx.h - SPACE_XL - ftr_h
        footer.render(ctx, SPACE_XL, footer_y, ctx.w - 2 * SPACE_XL, ftr_h)

        # ── Split: graph on the left, details panel on the right ─────────────
        # Wide pane → side-by-side. Narrow pane → graph fills, no details.
        # The details panel is always-visible and reflects the current
        # selection. No tooltip toggle, no Esc/Enter to manage.
        DETAILS_MIN_W = 280.0
        DETAILS_MAX_W = 480.0
        SPLIT_THRESHOLD = 700.0
        SEPARATOR_W = 1.0
        SEPARATOR_GAP = SPACE_MD
        if ctx.w >= SPLIT_THRESHOLD:
            details_w = max(DETAILS_MIN_W, min(DETAILS_MAX_W, ctx.w * 0.40))
            graph_right = ctx.w - details_w - SEPARATOR_GAP - SEPARATOR_W - SEPARATOR_GAP
        else:
            details_w = 0.0
            graph_right = ctx.w

        # ── Legend row (constrained to the graph half) ───────────────────────
        lanes_on_screen = self._lanes_on_screen()
        legend_y = y_cursor
        self._draw_legend(ctx, legend_y, lanes_on_screen, right_edge=graph_right)
        y_cursor = legend_y + LEGEND_H + LEGEND_PAD

        # ── Graph canvas ──────────────────────────────────────────────────────
        graph_y = y_cursor
        graph_h = footer_y - graph_y - SPACE_MD
        if graph_h <= 0:
            return
        self._draw_graph(ctx, graph_y, graph_h, right_edge=graph_right)

        # ── Vertical separator + details panel (right side) ───────────────────
        if details_w > 0:
            sep_x = graph_right + SEPARATOR_GAP
            ctx.rect(sep_x, y_cursor, SEPARATOR_W,
                     footer_y - y_cursor - SPACE_MD, HIGHLIGHT)
            details_x = sep_x + SEPARATOR_W + SEPARATOR_GAP
            self._draw_details_panel(ctx, details_x, y_cursor,
                                     details_w, footer_y - y_cursor - SPACE_MD)

        # ── Help overlay ──────────────────────────────────────────────────────
        if self._show_help:
            self._draw_help(ctx)

    # ── Legend ────────────────────────────────────────────────────────────────

    def _lanes_on_screen(self) -> list[tuple[int, str, str]]:
        """Return [(lane_idx, color, branch_label)] for lanes with commits."""
        seen: dict[int, tuple[str, str]] = {}
        for c in self._commits:
            lane = c["lane"]
            if lane not in seen:
                # Best label: find a matching ref name
                label = self._lane_label(lane, c)
                seen[lane] = (c["color"], label)
        return [(lane, color, label) for lane, (color, label) in sorted(seen.items())]

    def _lane_label(self, lane: int, sample_commit: dict) -> str:
        """Return a branch name for this lane, truncated to 12 chars."""
        # Look for a ref whose tip matches a commit in this lane
        lane_hashes = {c["hash"] for c in self._commits if c["lane"] == lane}
        for ref in self._refs:
            if ref["tip_hash"] in lane_hashes:
                name = ref["name"]
                return name[:12] + ("…" if len(name) > 12 else "")
        # Fallback: extract from the sample commit's refs list
        hint = gl._branch_name_from_refs(sample_commit.get("refs", []))
        if hint:
            return hint[:12] + ("…" if len(hint) > 12 else "")
        return f"lane {lane}"

    def _draw_legend(self, ctx: RenderContext, y: float,
                     lanes: list[tuple[int, str, str]],
                     right_edge: Optional[float] = None) -> None:
        if right_edge is None:
            right_edge = ctx.w
        x = PAD_X
        swatch_size = 10.0
        swatch_gap  = 6.0
        label_gap   = 18.0
        char_w = TEXT_CAPTION * 0.55
        max_item_w = swatch_size + swatch_gap + 12 * char_w + label_gap

        for lane_idx, color, label in lanes:
            # Wrap to next row if we'd overflow the legend region
            if x + max_item_w > right_edge - PAD_X and x > PAD_X:
                y += LEGEND_H
                x = PAD_X
            # Dim stale lanes
            draw_color = color
            for ref in self._refs:
                if ref["name"] == label or ref["name"][:12] == label[:12]:
                    if ref["is_stale"]:
                        draw_color = dim(color, 0x66)
                    break
            ctx.rect(x, y + (LEGEND_H - swatch_size) / 2, swatch_size, swatch_size,
                     fill=draw_color, radius=2.0)
            ctx.text(x + swatch_size + swatch_gap, y + (LEGEND_H - TEXT_CAPTION) / 2,
                     label, size=TEXT_CAPTION, color=draw_color)
            x += max_item_w

    # ── Graph ─────────────────────────────────────────────────────────────────

    def _max_lane(self) -> int:
        if not self._commits:
            return 0
        return max(c["lane"] for c in self._commits)

    def _lane_x(self, lane: int) -> float:
        return PAD_X + lane * LANE_W + LANE_W / 2

    def _draw_graph(self, ctx: RenderContext, graph_y: float, graph_h: float,
                    right_edge: Optional[float] = None) -> None:  # noqa: C901
        if right_edge is None:
            right_edge = ctx.w
        if not self._commits:
            # Empty viewport message — centred in the graph half, not the pane.
            cx = right_edge / 2
            cy = graph_y + graph_h / 2
            ctx.text(cx, cy, "No commits this week · press [ to go back",
                     size=TEXT_BODY, color=MUTED, align="center")
            return

        max_lane = self._max_lane()
        total_lane_w = (max_lane + 1) * LANE_W
        min_label_gutter = 180.0
        # Narrow graph region: collapse overflow lanes
        overflow_threshold = PAD_X + total_lane_w + min_label_gutter
        overflow_start_lane = None
        if overflow_threshold > right_edge:
            usable_w = right_edge - PAD_X - min_label_gutter
            max_visible_lanes = max(1, int(usable_w / LANE_W))
            if max_visible_lanes < max_lane + 1:
                overflow_start_lane = max_visible_lanes

        num_lanes = (overflow_start_lane or (max_lane + 1))
        label_x = PAD_X + num_lanes * LANE_W + 12.0

        # Build a hash → commit index map for edge rendering
        hash_to_idx: dict[str, int] = {c["hash"]: i for i, c in enumerate(self._commits)}

        # Assign y positions
        for i, c in enumerate(self._commits):
            c["y"] = graph_y + i * ROW_H + ROW_H / 2

        # ── Draw edges ────────────────────────────────────────────────────────
        for child_h, parent_h in self._edges:
            ci = hash_to_idx.get(child_h)
            pi = hash_to_idx.get(parent_h)
            if ci is None:
                continue
            child = self._commits[ci]
            child_lane = child["lane"]
            if overflow_start_lane and child_lane >= overflow_start_lane:
                continue
            child_x = self._lane_x(child_lane)
            child_y = child["y"]
            color = child["color"]

            if pi is None:
                # Parent is outside viewport — draw a stub going off the bottom
                stub_end_y = graph_y + graph_h
                ctx.line(child_x, child_y + NODE_R, child_x, stub_end_y,
                         color=dim(color, 0x88), width=2.0)
                continue

            parent = self._commits[pi]
            parent_lane = parent["lane"]
            if overflow_start_lane and parent_lane >= overflow_start_lane:
                continue
            parent_x = self._lane_x(parent_lane)
            parent_y = parent["y"]

            if child_lane == parent_lane:
                # Straight vertical edge
                ctx.line(child_x, child_y + NODE_R, parent_x, parent_y - NODE_R,
                         color=color, width=2.0)
            else:
                # Orthogonal polyline with 8px diagonal cuts
                self._draw_orthogonal_edge(
                    ctx, child_x, child_y + NODE_R, parent_x, parent_y - NODE_R, color
                )

        # ── Draw nodes ────────────────────────────────────────────────────────
        self._hit_nodes = []
        for i, c in enumerate(self._commits):
            lane = c["lane"]
            if overflow_start_lane and lane >= overflow_start_lane:
                # Collapsed overflow — render a small grey dot
                overflow_x = self._lane_x(overflow_start_lane - 1) + LANE_W
                cx_node = min(overflow_x, right_edge - PAD_X)
                cy_node = c["y"]
                color = MUTED
                ctx.circle(cx_node, cy_node, NODE_R - 2, color)
                self._hit_nodes.append((c["hash"], cx_node, cy_node, NODE_R))
                continue

            cx_node = self._lane_x(lane)
            cy_node = c["y"]
            color = c["color"]

            # Dim stale branches
            for ref in self._refs:
                if ref["is_stale"] and ref["tip_hash"] == c["hash"]:
                    color = dim(color, 0x66)
                    break

            selected = i == self._sel

            if selected:
                # Selection ring
                ctx.circle(cx_node, cy_node, NODE_R + 4, HIGHLIGHT)

            # HEAD ring
            if c["hash"] == self._head_sha:
                ctx.circle(cx_node, cy_node, HEAD_RING, ACCENT)

            if c["is_merge"]:
                # Merge: draw disc then inner BG disc to form a ring
                ctx.circle(cx_node, cy_node, NODE_R, color)
                ctx.circle(cx_node, cy_node, NODE_R - 2, BG)
            else:
                ctx.circle(cx_node, cy_node, NODE_R, color)

            self._hit_nodes.append((c["hash"], cx_node, cy_node, NODE_R))

            # ── Ref badges ───────────────────────────────────────────────────
            badge_x = label_x
            ref_badge_w = 0.0
            for ref in self._refs:
                if ref["tip_hash"] == c["hash"]:
                    self._draw_badge(ctx, badge_x, cy_node, ref["name"], color)
                    # Advance by an approximate width. The host sizes the badge
                    # from real font metrics so the rendered pill may differ slightly
                    # from this estimate — acceptable for horizontal flow spacing.
                    bw = self._approx_badge_w(ref["name"])
                    badge_x += bw + 4.0
                    ref_badge_w += bw + 4.0

            # ── Subject label ─────────────────────────────────────────────────
            subj_x = label_x + ref_badge_w
            subj_avail = right_edge - subj_x - PAD_X
            if subj_avail > 40:
                subj_color = FG if selected else FG
                ctx.text(subj_x, cy_node - TEXT_CAPTION / 2,
                         c["subject"], size=TEXT_CAPTION, color=subj_color,
                         max_width=subj_avail)

    def _draw_orthogonal_edge(self, ctx: RenderContext,
                              x1: float, y1: float,
                              x2: float, y2: float,
                              color: str) -> None:
        """Orthogonal polyline with 8px diagonal cuts for lane-change edges."""
        CUT = 8.0
        y_mid = y1 + (y2 - y1) / 2.0

        if abs(x1 - x2) < 1.0:
            ctx.line(x1, y1, x2, y2, color=color, width=2.0)
            return

        # 4-segment polyline:
        # vertical from (x1,y1) down to cut start → diagonal → horizontal → diagonal → vertical to (x2,y2)
        if x2 > x1:
            # Fork right
            ctx.line(x1, y1, x1, y_mid - CUT, color=color, width=2.0)
            ctx.line(x1, y_mid - CUT, x1 + CUT, y_mid, color=color, width=2.0)
            ctx.line(x1 + CUT, y_mid, x2 - CUT, y_mid, color=color, width=2.0)
            ctx.line(x2 - CUT, y_mid, x2, y_mid + CUT, color=color, width=2.0)
            ctx.line(x2, y_mid + CUT, x2, y2, color=color, width=2.0)
        else:
            # Merge left
            ctx.line(x1, y1, x1, y_mid - CUT, color=color, width=2.0)
            ctx.line(x1, y_mid - CUT, x1 - CUT, y_mid, color=color, width=2.0)
            ctx.line(x1 - CUT, y_mid, x2 + CUT, y_mid, color=color, width=2.0)
            ctx.line(x2 + CUT, y_mid, x2, y_mid + CUT, color=color, width=2.0)
            ctx.line(x2, y_mid + CUT, x2, y2, color=color, width=2.0)

    def _draw_badge(self, ctx: RenderContext, x: float, cy: float,
                    name: str, color: str) -> None:
        """Emit a host-measured badge DrawCommand for a branch/tag ref.

        Tag refs use RADIUS_SM; branch refs use RADIUS_MD.
        The host sizes the pill from real font metrics.
        """
        is_tag = name.startswith("tag:")
        fill = YELLOW if is_tag else color
        radius = RADIUS_SM if is_tag else RADIUS_MD
        badge(ctx, x, cy, name, fill=fill, fg=BG,
              font_size=TEXT_HINT, radius=radius)

    @staticmethod
    def _approx_badge_w(label: str) -> float:
        """Approximate badge pixel width for horizontal flow cursor advance.

        Uses the same heuristic as the old badge() — good enough for spacing
        between consecutive badges. The host renders each badge precisely.
        """
        text_w = len(label) * TEXT_HINT * 0.62
        return max(32.0, text_w + 8.0 * 2)

    # ── Details panel ─────────────────────────────────────────────────────────
    # Always-visible right-side panel showing the currently-selected commit.
    # Replaces the old tooltip — selection just IS, the panel reflects it.
    # No toggle state, no Enter/Esc dance, no overlap with the graph.

    def _draw_details_panel(self, ctx: RenderContext, x: float, y: float,
                            w: float, h: float) -> None:
        # Empty state
        if not self._commits or not (0 <= self._sel < len(self._commits)):
            ctx.text(x + w / 2, y + h / 2,
                     "Select a commit (j/k or click)",
                     size=TEXT_CAPTION, color=MUTED, align="center")
            return

        c = self._commits[self._sel]
        INNER_PAD = 0.0  # x/y are already inside the separator gutter
        LINE_H = TEXT_BODY + 4.0
        avail_w = w - INNER_PAD * 2

        import datetime
        dt = datetime.datetime.fromtimestamp(c["ts"]).strftime("%Y-%m-%d %H:%M")

        tx = x + INNER_PAD
        ty = y + INNER_PAD

        # Short hash (monospace, accent)
        ctx.text(tx, ty, c["short_hash"], size=TEXT_CAPTION,
                 color=ACCENT, monospace=True)
        ty += TEXT_CAPTION + 6.0

        # Author + ISO timestamp (muted)
        ctx.text(tx, ty, f"{c['author']}  ·  {dt}",
                 size=TEXT_CAPTION, color=MUTED,
                 max_width=avail_w)
        ty += LINE_H + 4.0

        # Subject (bold, FG, wrapped up to 8 lines)
        subject_lines = _wrap_to_width(
            c["subject"], avail_w, TEXT_BODY, max_lines=8
        )
        for line in subject_lines:
            ctx.text(tx, ty, line, size=TEXT_BODY, color=FG, bold=True,
                     max_width=avail_w)
            ty += LINE_H
        ty += 6.0

        # Stats line (+added / −removed)
        if self._stats_unavailable:
            ctx.text(tx, ty, "stats unavailable",
                     size=TEXT_CAPTION, color=MUTED)
        else:
            ctx.text(tx, ty, f"+{c['added']}",
                     size=TEXT_CAPTION, color=GREEN)
            ctx.text(tx + 60.0, ty, f"−{c['removed']}",
                     size=TEXT_CAPTION, color=RED)
        ty += LINE_H + 8.0

        # Refs at this commit (if any)
        if c.get("refs"):
            ctx.text(tx, ty, "refs:", size=TEXT_CAPTION, color=MUTED)
            ty += TEXT_CAPTION + 2.0
            ref_text = ", ".join(r for r in c["refs"] if r and r != "HEAD")
            if ref_text:
                ctx.text(tx, ty, ref_text, size=TEXT_CAPTION, color=FG,
                         max_width=avail_w)
                ty += LINE_H + 6.0

        # Parents
        if c.get("parents"):
            ctx.text(tx, ty, "parents:", size=TEXT_CAPTION, color=MUTED)
            ty += TEXT_CAPTION + 2.0
            for p in c["parents"][:4]:
                ctx.text(tx, ty, p[:12], size=TEXT_CAPTION, color=FG,
                         monospace=True, max_width=avail_w)
                ty += LINE_H

    # ── Help overlay ──────────────────────────────────────────────────────────

    def _draw_help(self, ctx: RenderContext) -> None:
        help_rows = [
            (["[", "]"],  "go to older / newer week"),
            (["t"],       "jump to today"),
            (["j", "/", "k"], "select next / previous commit"),
            (["h", "/", "l"], "select commit in left / right lane"),
            (["g", "/", "G"], "first / last commit"),
            (["c"],       "copy hash (shown in status bar)"),
            (["o"],       "open on GitHub (shown in status bar)"),
            (["r"],       "force refresh"),
            (["?"],       "toggle this help"),
        ]
        W = min(400.0, ctx.w - 48.0)
        card = Card([KeyRow(keys, desc) for keys, desc in help_rows])
        card_h = card.measure(W)
        bx = (ctx.w - W) / 2
        by = max(8.0, (ctx.h - card_h) / 2)
        card.render(ctx, bx, by, W, card_h)

    # ── Utilities ─────────────────────────────────────────────────────────────

    @staticmethod
    def _format_week(start_ts: int, end_ts: int) -> str:
        import datetime
        start = datetime.datetime.fromtimestamp(start_ts)
        end   = datetime.datetime.fromtimestamp(end_ts)
        fmt = "%b %-d"
        return f"{start.strftime(fmt)} – {end.strftime(fmt)}"


if __name__ == "__main__":
    CommitGraphApp().run()
