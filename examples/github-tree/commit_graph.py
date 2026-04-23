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
    Column, Card, Header, Spacer, Footer, Label, KeyRow,
    TEXT_CAPTION, TEXT_BODY, TEXT_HINT, TEXT_HEADING,
    SPACE_MD, SPACE_LG, SPACE_XL,
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
            root = gl.find_repo_root(cwd)
            if root is None:
                self._set_mode(MODE_NO_REPO)
                return
            self._repo_root = root
            self._repo_name = os.path.basename(root)
            self._head_branch = gl.get_head_branch(root)
            self._origin_url = gl.get_origin_remote(root)
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
            # Refs + commits fetched; numstats can run in parallel but we keep
            # it simple (GIL-safe subprocess calls) — sequential is fine.
            refs = gl.fetch_refs(self._repo_root, now_ts)
            commits = gl.fetch_commits(self._repo_root, start_ts, end_ts)

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
            self.emit.schedule_render(after_ms=16)
            return

        if self._show_help:
            # Any key dismisses help
            self._show_help = False
            self.emit.schedule_render(after_ms=16)
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
            self.emit.schedule_render(after_ms=16)
        elif key in ("k", "up") and n > 0:
            self._sel = max(self._sel - 1, 0)
            self.emit.schedule_render(after_ms=16)
        elif key in ("h", "left") and n > 0:
            # Move to commit on left neighbouring lane at the same row
            cur_lane = self._commits[self._sel]["lane"] if n > 0 else 0
            target_lane = cur_lane - 1
            if target_lane >= 0:
                best = self._nearest_in_lane(target_lane)
                if best is not None:
                    self._sel = best
                    self.emit.schedule_render(after_ms=16)
        elif key in ("l", "right") and n > 0:
            cur_lane = self._commits[self._sel]["lane"] if n > 0 else 0
            target_lane = cur_lane + 1
            best = self._nearest_in_lane(target_lane)
            if best is not None:
                self._sel = best
                self.emit.schedule_render(after_ms=16)
        elif key == "g":
            self._sel = 0
            self.emit.schedule_render(after_ms=16)
        elif key == "G":
            self._sel = max(0, n - 1)
            self.emit.schedule_render(after_ms=16)
        elif key == "c":
            # IMPL-NOTE: SDK has no clipboard emit — log + status line instead.
            if n > 0:
                full_hash = self._commits[self._sel]["hash"]
                self.emit.info(f"commit-graph: copy hash {full_hash}")
                self.emit.status_summary(f"Hash: {full_hash}")
                self.emit.schedule_render(after_ms=16)
        elif key == "o":
            # Open commit on GitHub if origin is a GitHub remote
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
                # Find commit index by hash
                for j, c in enumerate(self._commits):
                    if c["hash"] == h:
                        self._sel = j
                        self._show_help = False
                        self.emit.schedule_render(after_ms=16)
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
        from plexi_sdk.ui import _truncate_to_width
        week_str = self._format_week(start_ts, end_ts)
        n = len(self._commits)
        subtitle = f"{self._repo_name} · {week_str} · {n} commit{'s' if n != 1 else ''}"
        if self._week_offset > 0:
            subtitle += f" (–{self._week_offset}w)"

        # Measure and render Header manually so we can position the rest below it
        from plexi_sdk.ui import Header as UIHeader
        header = UIHeader(title="Commit Graph", subtitle=subtitle)
        hdr_h = header.measure(ctx.w - 2 * SPACE_XL)
        header.render(ctx, SPACE_XL, SPACE_XL, ctx.w - 2 * SPACE_XL, hdr_h)
        y_cursor = SPACE_XL + hdr_h + SPACE_MD

        # ── Footer ────────────────────────────────────────────────────────────
        from plexi_sdk.ui import Footer as UIFooter
        footer = UIFooter("[ ] week  t today  j k commit  h l lane  g G ends  c copy  o open  r refresh  ? help")
        ftr_h = footer.measure(ctx.w - 2 * SPACE_XL)
        footer_y = ctx.h - SPACE_XL - ftr_h
        footer.render(ctx, SPACE_XL, footer_y, ctx.w - 2 * SPACE_XL, ftr_h)

        # ── Legend row ────────────────────────────────────────────────────────
        lanes_on_screen = self._lanes_on_screen()
        legend_y = y_cursor
        self._draw_legend(ctx, legend_y, lanes_on_screen)
        y_cursor = legend_y + LEGEND_H + LEGEND_PAD

        # ── Graph canvas ──────────────────────────────────────────────────────
        graph_y = y_cursor
        graph_h = footer_y - graph_y - SPACE_MD
        if graph_h <= 0:
            return

        self._draw_graph(ctx, graph_y, graph_h)

        # ── Tooltip ───────────────────────────────────────────────────────────
        if self._commits and 0 <= self._sel < len(self._commits):
            self._draw_tooltip(ctx)

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
                     lanes: list[tuple[int, str, str]]) -> None:
        x = PAD_X
        swatch_size = 10.0
        swatch_gap  = 6.0
        label_gap   = 18.0
        char_w = TEXT_CAPTION * 0.55
        max_item_w = swatch_size + swatch_gap + 12 * char_w + label_gap

        for lane_idx, color, label in lanes:
            # Wrap to next row if we'd overflow
            if x + max_item_w > ctx.w - PAD_X and x > PAD_X:
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

    def _draw_graph(self, ctx: RenderContext, graph_y: float, graph_h: float) -> None:  # noqa: C901
        if not self._commits:
            # Empty viewport message
            cx = ctx.w / 2
            cy = graph_y + graph_h / 2
            ctx.text(cx, cy, "No commits this week · press [ to go back",
                     size=TEXT_BODY, color=MUTED, align="center")
            return

        max_lane = self._max_lane()
        total_lane_w = (max_lane + 1) * LANE_W
        min_label_gutter = 180.0
        # Narrow pane: collapse overflow lanes
        overflow_threshold = PAD_X + total_lane_w + min_label_gutter
        overflow_start_lane = None
        if overflow_threshold > ctx.w:
            usable_w = ctx.w - PAD_X - min_label_gutter
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
                cx_node = min(overflow_x, ctx.w - PAD_X)
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
            avail_label_w = ctx.w - label_x - PAD_X
            ref_badge_w = 0.0
            for ref in self._refs:
                if ref["tip_hash"] == c["hash"]:
                    bw = self._draw_badge(ctx, badge_x, cy_node, ref["name"], color)
                    badge_x += bw + 4.0
                    ref_badge_w += bw + 4.0

            # ── Subject label ─────────────────────────────────────────────────
            subj_x = label_x + ref_badge_w
            subj_avail = ctx.w - subj_x - PAD_X
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
                    name: str, color: str) -> float:
        """Draw a branch-name pill badge. Returns badge width."""
        BADGE_PAD_H = 5.0
        BADGE_PAD_V = 3.0
        fs = TEXT_CAPTION - 1.0
        char_w = fs * 0.55
        max_badge_chars = 16
        label = name[:max_badge_chars] + ("…" if len(name) > max_badge_chars else "")
        bw = len(label) * char_w + BADGE_PAD_H * 2
        bh = fs + BADGE_PAD_V * 2

        is_tag = name.startswith("tag:")
        fill = YELLOW if is_tag else color
        radius = 2.0 if is_tag else 8.0

        ctx.rect(x, cy - bh / 2, bw, bh, fill=fill, radius=radius)
        ctx.text(x + BADGE_PAD_H, cy - fs / 2, label, size=fs, color=BG)
        return bw

    # ── Tooltip ───────────────────────────────────────────────────────────────

    def _draw_tooltip(self, ctx: RenderContext) -> None:
        c = self._commits[self._sel]
        lane = c["lane"]
        cx_node = self._lane_x(lane)
        cy_node = c["y"]

        TIP_W = min(360.0, ctx.w - 48.0)
        INNER_PAD = 12.0
        LINE_H = TEXT_BODY + 4.0

        # Estimate content height
        import time as _t
        import datetime
        dt = datetime.datetime.fromtimestamp(c["ts"]).strftime("%Y-%m-%d %H:%M")
        stats_line = (
            "stats unavailable (repo too large)"
            if self._stats_unavailable
            else f"+{c['added']}  -{c['removed']}"
        )

        # Wrap subject (up to 6 lines)
        from plexi_sdk.ui import _wrap_to_width
        subject_lines = _wrap_to_width(
            c["subject"], TIP_W - INNER_PAD * 2, TEXT_BODY, max_lines=6
        )
        content_h = (
            TEXT_CAPTION      # hash line
            + LINE_H          # author + date
            + LINE_H * max(1, len(subject_lines))
            + LINE_H          # stats
            + INNER_PAD * 2
        )

        # Position: anchor to node, offset right; flip if needed
        tip_x = cx_node + 16.0
        tip_y = cy_node - 8.0
        if tip_x + TIP_W > ctx.w - 8:
            tip_x = cx_node - TIP_W - 16.0
        if tip_y + content_h > ctx.h - 8:
            tip_y = cy_node - content_h - 8.0
        tip_x = max(8.0, tip_x)
        tip_y = max(8.0, tip_y)

        # Border (1px HIGHLIGHT behind)
        ctx.rect(tip_x - 1, tip_y - 1, TIP_W + 2, content_h + 2,
                 fill=HIGHLIGHT, radius=9.0)
        ctx.rect(tip_x, tip_y, TIP_W, content_h,
                 fill=SURFACE, radius=8.0)

        tx = tip_x + INNER_PAD
        ty = tip_y + INNER_PAD

        # Short hash
        ctx.text(tx, ty, c["short_hash"], size=TEXT_CAPTION,
                 color=ACCENT, monospace=True)
        ty += TEXT_CAPTION + 4.0

        # Author + date
        ctx.text(tx, ty, f"{c['author']}  ·  {dt}",
                 size=TEXT_CAPTION, color=MUTED,
                 max_width=TIP_W - INNER_PAD * 2)
        ty += LINE_H

        # Subject lines
        for line in subject_lines:
            ctx.text(tx, ty, line, size=TEXT_BODY, color=FG,
                     max_width=TIP_W - INNER_PAD * 2)
            ty += LINE_H

        # Stats
        if not self._stats_unavailable:
            ctx.text(tx, ty, f"+{c['added']}", size=TEXT_CAPTION, color=GREEN)
            ctx.text(tx + 55.0, ty, f"-{c['removed']}", size=TEXT_CAPTION, color=RED)
        else:
            ctx.text(tx, ty, "stats unavailable", size=TEXT_CAPTION, color=MUTED)

    # ── Help overlay ──────────────────────────────────────────────────────────

    def _draw_help(self, ctx: RenderContext) -> None:
        help_items = [
            ("[ ]",  "go to older / newer week"),
            ("t",    "jump to today"),
            ("j / k", "select next / previous commit"),
            ("h / l", "select commit in left / right lane"),
            ("g / G", "first / last commit"),
            ("c",    "copy commit hash (shown in status bar)"),
            ("o",    "open on GitHub (shown in status bar)"),
            ("r",    "force refresh"),
            ("?",    "toggle this help"),
        ]
        W = min(380.0, ctx.w - 48.0)
        ROW = 22.0
        H = ROW * len(help_items) + 24.0
        bx = (ctx.w - W) / 2
        by = (ctx.h - H) / 2

        ctx.rect(bx - 1, by - 1, W + 2, H + 2, fill=HIGHLIGHT, radius=9.0)
        ctx.rect(bx, by, W, H, fill=SURFACE, radius=8.0)

        ty = by + 12.0
        for key_str, desc in help_items:
            ctx.text(bx + 12.0, ty, key_str, size=TEXT_CAPTION,
                     color=ACCENT, monospace=True)
            ctx.text(bx + 80.0, ty, desc, size=TEXT_CAPTION, color=FG)
            ty += ROW

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
