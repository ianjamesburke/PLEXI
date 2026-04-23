#!/usr/bin/env python3
from __future__ import annotations

"""Commit Graph v2 — viewport-scoped lanes, fixed label column, merge diamonds.

Key changes from v1:
- Lanes allocated only for refs with commits in the visible week (§2).
- Hard cap: 5 lanes; overflow collapses to a single "other" lane (§2).
- Fixed right-hand label column with hard-clip via truncate_to_width (§3).
- One badge per row; overflow refs go to tooltip "also:" line (§3).
- Merge commits drawn as hollow diamonds (§5).
- Empty-graph: centred Card, not raw text (§7).
- Tooltip clamped so it never renders off-pane (§4).
- Esc clears tooltip; empty-canvas click clears selection+tooltip (§4).
- Footer simplified; full key list lives in ? overlay (§6).
"""

import os
import threading
import time
from typing import Optional

from plexi_sdk import App, RenderContext, dim, truncate_to_width
from plexi_sdk import BG, SURFACE, HIGHLIGHT, ACCENT, MUTED, FG, RED, GREEN, YELLOW
from plexi_sdk.ui import (
    Column, Card, Header, Spacer, Footer, Label, KeyRow,
    TEXT_CAPTION, TEXT_BODY, TEXT_HINT, TEXT_HEADING,
    SPACE_MD, SPACE_LG, SPACE_XL,
    _truncate_to_width, _char_px,
)

import git_log as gl

# ── Layout constants ──────────────────────────────────────────────────────────
PAD_X        = 24.0
LANE_W       = 28.0
ROW_H        = 28.0
NODE_R       = 5.0
HEAD_RING    = 8.0
LEGEND_H     = 22.0
LEGEND_PAD   = 8.0
BADGE_GUTTER = 8.0    # gap between badge right edge and subject text
TIP_W_MAX    = 360.0

# ── App modes ─────────────────────────────────────────────────────────────────
MODE_LOADING = "loading"
MODE_NO_REPO = "no_repo"
MODE_NO_GIT  = "no_git"
MODE_READY   = "ready"
MODE_ERROR   = "error"

SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]

POLL_INTERVAL_S = 3.0

MAX_LANES  = 5   # lanes 0–4 render normally; lane 5 = collapse
OTHER_LANE = 5


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

        self._week_offset: int = 0

        self._commits: list[dict] = []
        self._refs: list[dict] = []
        self._edges: list[tuple[str, str]] = []
        self._stats_unavailable: bool = False

        self._sel: int = 0
        self._tooltip_visible: bool = False
        self._show_help: bool = False

        # Hit-test lists rebuilt each render
        self._hit_nodes: list[tuple[str, float, float, float]] = []
        # Label-row hit strips: [(commit_hash, strip_y_top, strip_y_bottom)]
        self._hit_labels: list[tuple[str, float, float]] = []
        # Canvas rect for empty-click detection: (x, y, w, h)
        self._graph_canvas_rect: tuple[float, float, float, float] = (0, 0, 0, 0)

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
            refs = gl.fetch_refs(self._repo_root, now_ts)
            self.emit.debug(f"fetch_graph: fetched {len(refs)} refs")
            commits = gl.fetch_commits(self._repo_root, start_ts, end_ts)
            self.emit.debug(f"fetch_graph: fetched {len(commits)} commits in window")

            gl.assign_lanes(commits, refs)

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
            self._tooltip_visible = False

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

    # ── Polling ───────────────────────────────────────────────────────────────

    def _maybe_poll(self) -> None:
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

        if key == "?":
            self._show_help = not self._show_help
            self.emit.schedule_render(after_ms=16)
            return

        if self._show_help:
            self._show_help = False
            self.emit.schedule_render(after_ms=16)
            return

        # Esc clears tooltip; keeps _sel for j/k continuity (§4)
        if key in ("escape", "Escape"):
            self._tooltip_visible = False
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
            self._tooltip_visible = True
            self.emit.schedule_render(after_ms=16)
        elif key in ("k", "up") and n > 0:
            self._sel = max(self._sel - 1, 0)
            self._tooltip_visible = True
            self.emit.schedule_render(after_ms=16)
        elif key in ("h", "left") and n > 0:
            cur_lane = self._commits[self._sel]["lane"] if n > 0 else 0
            best = self._nearest_in_lane(cur_lane - 1)
            if best is not None:
                self._sel = best
                self.emit.schedule_render(after_ms=16)
        elif key in ("l", "right") and n > 0:
            cur_lane = self._commits[self._sel]["lane"] if n > 0 else 0
            best = self._nearest_in_lane(cur_lane + 1)
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
            if n > 0:
                full_hash = self._commits[self._sel]["hash"]
                self.emit.info(f"commit-graph: copy hash {full_hash}")
                self.emit.status_summary(f"Hash: {full_hash}")
                self.emit.schedule_render(after_ms=16)
        elif key == "o":
            self._open_on_github()
        elif key == "r":
            threading.Thread(target=self._fetch_graph, daemon=True).start()

    def _nearest_in_lane(self, lane: int) -> Optional[int]:
        if not self._commits:
            return None
        cur_idx = self._sel
        candidates = [i for i, c in enumerate(self._commits) if c["lane"] == lane]
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
        self.emit.info(f"commit-graph: open {gh_url}")
        self.emit.status_summary(f"URL: {gh_url}")
        self.emit.schedule_render(after_ms=16)

    def on_click(self, ctx: RenderContext, x: float, y: float, button: str) -> None:
        if self._mode != MODE_READY:
            return

        # Hit-test precedence: node > label-row > empty canvas (§4)
        # 1. Nodes
        for i, (h, cx, cy, r) in enumerate(self._hit_nodes):
            if (x - cx) ** 2 + (y - cy) ** 2 <= (r + 4) ** 2:
                for j, c in enumerate(self._commits):
                    if c["hash"] == h:
                        self._sel = j
                        self._tooltip_visible = True
                        self._show_help = False
                        self.emit.schedule_render(after_ms=16)
                        return
                break

        # 2. Label rows
        for h, y_top, y_bot in self._hit_labels:
            if y_top <= y <= y_bot:
                for j, c in enumerate(self._commits):
                    if c["hash"] == h:
                        self._sel = j
                        self._tooltip_visible = True
                        self._show_help = False
                        self.emit.schedule_render(after_ms=16)
                        return
                break

        # 3. Empty canvas click — clear tooltip, keep _sel (§4)
        gx, gy, gw, gh = self._graph_canvas_rect
        if gx <= x <= gx + gw and gy <= y <= gy + gh:
            self._tooltip_visible = False
            self.emit.schedule_render(after_ms=16)

    # ── Render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:  # noqa: C901
        self._maybe_poll()

        if self._mode == MODE_LOADING:
            self._render_loading(ctx)
            self.emit.schedule_render(after_ms=100)
            return

        if self._mode == MODE_NO_REPO:
            self._render_error_state(
                ctx,
                title="Not a git repository",
                lines=["Launch from a repo terminal.", "Press r to retry."],
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
            Footer("[ ] week  r refresh  ? help"),
        ]))

    def _render_ready(self, ctx: RenderContext) -> None:
        ctx.clear(BG)

        import time as _time
        now_ts = int(_time.time())
        end_ts = now_ts - self._week_offset * 7 * 86400
        start_ts = end_ts - 7 * 86400

        # ── Header (§6) ───────────────────────────────────────────────────────
        n = len(self._commits)
        subtitle = f"{self._repo_name} · week · {n} commit{'s' if n != 1 else ''}"

        from plexi_sdk.ui import Header as UIHeader
        header = UIHeader(title="Commit Graph", subtitle=subtitle)
        hdr_h = header.measure(ctx.w - 2 * SPACE_XL)
        header.render(ctx, SPACE_XL, SPACE_XL, ctx.w - 2 * SPACE_XL, hdr_h)
        y_cursor = SPACE_XL + hdr_h + SPACE_MD

        # ── Footer (§6) ───────────────────────────────────────────────────────
        from plexi_sdk.ui import Footer as UIFooter
        footer = UIFooter("[ ] week  j k select  esc clear  c copy  r refresh  ? help")
        ftr_h = footer.measure(ctx.w - 2 * SPACE_XL)
        footer_y = ctx.h - SPACE_XL - ftr_h
        footer.render(ctx, SPACE_XL, footer_y, ctx.w - 2 * SPACE_XL, ftr_h)

        # ── Legend (§6) ───────────────────────────────────────────────────────
        legend_y = y_cursor
        lanes_on_screen = self._lanes_on_screen()
        if lanes_on_screen:
            self._draw_legend(ctx, legend_y, lanes_on_screen)
            y_cursor = legend_y + LEGEND_H + LEGEND_PAD
        else:
            y_cursor = legend_y

        # ── Graph canvas ──────────────────────────────────────────────────────
        graph_y = y_cursor
        graph_h = footer_y - graph_y - SPACE_MD
        if graph_h <= 0:
            return

        self._graph_canvas_rect = (0.0, graph_y, ctx.w, graph_h)

        if not self._commits:
            self._render_empty_state(ctx, graph_y, graph_h)
        else:
            rows = self._render_graph_canvas(
                ctx, PAD_X, graph_y, ctx.w - 2 * PAD_X, graph_h
            )
            self._render_labels_column(ctx, rows, graph_y, graph_h)

        # ── Tooltip ───────────────────────────────────────────────────────────
        if self._tooltip_visible and self._commits and 0 <= self._sel < len(self._commits):
            self._render_tooltip(ctx)

        # ── Help overlay ──────────────────────────────────────────────────────
        if self._show_help:
            self._draw_help(ctx)

    # ── Empty state (§7) ──────────────────────────────────────────────────────

    def _render_empty_state(self, ctx: RenderContext, graph_y: float, graph_h: float) -> None:
        total_branches = len(self._refs)
        CARD_W = min(340.0, ctx.w - 48.0)
        CARD_H = 96.0
        cx = (ctx.w - CARD_W) / 2
        cy = graph_y + (graph_h - CARD_H) / 2

        ctx.rect(cx - 1, cy - 1, CARD_W + 2, CARD_H + 2, fill=HIGHLIGHT, radius=9.0)
        ctx.rect(cx, cy, CARD_W, CARD_H, fill=SURFACE, radius=8.0)

        ctx.text(cx + CARD_W / 2, cy + 20.0, "No commits this week",
                 size=TEXT_BODY, color=FG, align="top_center", bold=True)
        ctx.text(cx + CARD_W / 2, cy + 44.0,
                 f"{self._repo_name} · {total_branches} branches tracked",
                 size=TEXT_CAPTION, color=MUTED, align="top_center")
        ctx.text(cx + CARD_W / 2, cy + 68.0,
                 "press [ to view previous week",
                 size=TEXT_HINT, color=MUTED, align="top_center")

    # ── Graph canvas (§8 / §3) ────────────────────────────────────────────────

    def _render_graph_canvas(
        self, ctx: RenderContext,
        graph_x: float, graph_y: float, graph_w: float, graph_h: float,
    ) -> list[tuple[int, float]]:
        """Draw lanes, edges, and nodes. Returns [(commit_idx, row_y)] list."""
        commits = self._commits
        n = len(commits)
        if n == 0:
            return []

        # Row height (§3)
        row_h = ROW_H
        if n * ROW_H < graph_h * 0.6 and graph_h > 400:
            row_h = min(36.0, graph_h / n)

        max_visible = max(1, int(graph_h / row_h))
        truncated = n > max_visible
        visible_count = min(n, max_visible - 1 if truncated else max_visible)

        # Compute lane region width (§3)
        num_drawn_lanes = max(1, len(set(
            min(c["lane"], OTHER_LANE) for c in commits[:visible_count]
        )))
        # OTHER_LANE occupies one slot
        lane_region_w = min(num_drawn_lanes * LANE_W + 16.0, 0.35 * ctx.w)

        # Assign y positions
        for i, c in enumerate(commits):
            c["y"] = graph_y + i * row_h + row_h / 2

        hash_to_idx: dict[str, int] = {c["hash"]: i for i, c in enumerate(commits)}

        def lane_x(lane: int) -> float:
            effective = min(lane, OTHER_LANE)
            return graph_x + effective * LANE_W + LANE_W / 2

        # ── Edges ─────────────────────────────────────────────────────────────
        for child_h, parent_h in self._edges:
            ci = hash_to_idx.get(child_h)
            pi = hash_to_idx.get(parent_h)
            if ci is None or ci >= visible_count:
                continue
            child = commits[ci]
            child_cx = lane_x(child["lane"])
            child_cy = child["y"]

            # Mainline vs off-mainline edge colour (§5)
            is_mainline = (parent_h == child["parents"][0]) if child["parents"] else True

            if pi is None:
                ctx.line(child_cx, child_cy + NODE_R, child_cx, graph_y + graph_h,
                         color=dim(child["color"], 0x88), width=2.0)
                continue
            if pi >= visible_count:
                continue

            parent = commits[pi]
            parent_cx = lane_x(parent["lane"])
            parent_cy = parent["y"]

            edge_color = child["color"] if is_mainline else parent["color"]

            if child["lane"] == parent["lane"]:
                ctx.line(child_cx, child_cy + NODE_R, parent_cx, parent_cy - NODE_R,
                         color=edge_color, width=2.0)
            else:
                self._draw_orthogonal_edge(
                    ctx, child_cx, child_cy + NODE_R,
                    parent_cx, parent_cy - NODE_R, edge_color,
                )

        # ── Nodes ─────────────────────────────────────────────────────────────
        self._hit_nodes = []
        rows: list[tuple[int, float]] = []

        for i in range(visible_count):
            c = commits[i]
            cx_node = lane_x(c["lane"])
            cy_node = c["y"]
            color = c["color"]

            for ref in self._refs:
                if ref["is_stale"] and ref["tip_hash"] == c["hash"]:
                    color = dim(color, 0x66)
                    break

            selected = (i == self._sel)

            if selected:
                ctx.circle(cx_node, cy_node, NODE_R + 4, HIGHLIGHT)

            if c["hash"] == self._head_sha:
                ctx.circle(cx_node, cy_node, HEAD_RING, ACCENT)

            if c["is_merge"]:
                self._draw_merge_node(ctx, cx_node, cy_node, color)
            else:
                ctx.circle(cx_node, cy_node, NODE_R, color)

            self._hit_nodes.append((c["hash"], cx_node, cy_node, NODE_R))
            rows.append((i, cy_node))

        # Truncation stub (§3)
        if truncated:
            remaining = n - visible_count
            stub_y = graph_y + visible_count * row_h + row_h / 2
            ctx.text(
                graph_x, stub_y,
                f"… +{remaining} older this week — press [ to view",
                size=TEXT_CAPTION, color=MUTED,
            )

        return rows

    # ── Label column (§3 / §8) ────────────────────────────────────────────────

    def _render_labels_column(
        self, ctx: RenderContext,
        rows: list[tuple[int, float]],
        graph_y: float, graph_h: float,
    ) -> None:
        """Draw badges + truncated subjects. Uses truncate_to_width for hard clipping."""
        commits = self._commits

        num_drawn_lanes = max(1, len(set(
            min(c["lane"], OTHER_LANE) for c in commits
        )))
        lane_region_w = min(num_drawn_lanes * LANE_W + 16.0, 0.35 * ctx.w)
        label_x = PAD_X + lane_region_w + 12.0
        label_w = ctx.w - label_x - PAD_X

        self._hit_labels = []

        for commit_idx, cy_node in rows:
            c = commits[commit_idx]
            color = c["color"]
            selected = (commit_idx == self._sel)

            # ── Badge: one per row, overflow into tooltip "also:" ─────────────
            refs_for_commit = [r for r in self._refs if r["tip_hash"] == c["hash"]]
            max_badge_w = min(120.0, label_w * 0.4)
            badge_w = 0.0

            if refs_for_commit:
                ref = refs_for_commit[0]
                overflow_count = len(refs_for_commit) - 1
                badge_label = ref["name"]
                if overflow_count > 0:
                    badge_label = f"{ref['name']} +{overflow_count}"
                # IMPL-NOTE: truncate_to_width is from plexi_sdk.__init__ (not ui)
                badge_label = truncate_to_width(badge_label, max_badge_w, TEXT_CAPTION)
                if badge_label:
                    badge_w = self._draw_badge(ctx, label_x, cy_node, badge_label, color)

            # ── Subject label ─────────────────────────────────────────────────
            subj_x = label_x + (badge_w + BADGE_GUTTER if badge_w > 0 else 0.0)
            subj_avail = ctx.w - subj_x - PAD_X
            if subj_avail > 30:
                # truncate_to_width guarantees no overflow past ctx.w - PAD_X
                subject = truncate_to_width(c["subject"], subj_avail, TEXT_CAPTION)
                if subject:
                    ctx.text(subj_x, cy_node - TEXT_CAPTION / 2,
                             subject, size=TEXT_CAPTION, color=FG)

            # ── Label-row hit strip ───────────────────────────────────────────
            self._hit_labels.append((
                c["hash"],
                cy_node - ROW_H / 2,
                cy_node + ROW_H / 2,
            ))

    # ── Merge node helper (§5) ────────────────────────────────────────────────

    def _draw_merge_node(self, ctx: RenderContext,
                         cx: float, cy: float, color: str) -> None:
        """Hollow diamond 10×10 px, 2px stroke in lane colour."""
        D = 5.0
        ctx.line(cx,     cy - D, cx + D, cy,     color=color, width=2.0)
        ctx.line(cx + D, cy,     cx,     cy + D, color=color, width=2.0)
        ctx.line(cx,     cy + D, cx - D, cy,     color=color, width=2.0)
        ctx.line(cx - D, cy,     cx,     cy - D, color=color, width=2.0)

    # ── Legend (§6) ───────────────────────────────────────────────────────────

    def _lanes_on_screen(self) -> list[tuple[int, str, str]]:
        """Return [(lane_idx, color, label)] for lanes with commits in window."""
        seen: dict[int, tuple[str, str]] = {}
        other_count = 0
        for c in self._commits:
            lane = c["lane"]
            if lane == OTHER_LANE:
                other_count += 1
                continue
            if lane not in seen:
                seen[lane] = (c["color"], self._lane_label(lane, c))

        result = [(lane, color, label) for lane, (color, label) in sorted(seen.items())]
        if other_count > 0:
            result.append((OTHER_LANE, MUTED, "other"))
        return result

    def _lane_label(self, lane: int, sample_commit: dict) -> str:
        lane_hashes = {c["hash"] for c in self._commits if c["lane"] == lane}
        for ref in self._refs:
            if ref["tip_hash"] in lane_hashes:
                name = ref["name"]
                return name[:14] + ("…" if len(name) > 14 else "")
        rname = sample_commit.get("_ref")
        if rname:
            return rname[:14] + ("…" if len(rname) > 14 else "")
        return f"lane {lane}"

    def _draw_legend(self, ctx: RenderContext, y: float,
                     lanes: list[tuple[int, str, str]]) -> None:
        """Single-line legend; shrinks names before wrapping (§6)."""
        x = PAD_X
        swatch_size = 10.0
        swatch_gap  = 6.0
        label_gap   = 14.0
        right_edge  = ctx.w - PAD_X

        # Try progressively shorter name caps until everything fits on one row
        max_name_chars = 14
        for cap in (14, 12, 10, 8):
            char_w = TEXT_CAPTION * 0.55
            max_item_w = swatch_size + swatch_gap + cap * char_w + label_gap
            if PAD_X + len(lanes) * max_item_w <= right_edge or cap == 8:
                max_name_chars = cap
                break

        char_w = TEXT_CAPTION * 0.55

        for lane_idx, color, label in lanes:
            if len(label) > max_name_chars:
                label = label[:max_name_chars - 1] + "…"

            item_w = swatch_size + swatch_gap + len(label) * char_w + label_gap

            if x + item_w > right_edge and x > PAD_X:
                y += LEGEND_H
                x = PAD_X

            draw_color = color
            if lane_idx < OTHER_LANE:
                for ref in self._refs:
                    if ref["name"].startswith(label.rstrip("…")):
                        if ref["is_stale"]:
                            draw_color = dim(color, 0x66)
                        break

            ctx.rect(x, y + (LEGEND_H - swatch_size) / 2, swatch_size, swatch_size,
                     fill=draw_color, radius=2.0)
            ctx.text(x + swatch_size + swatch_gap, y + (LEGEND_H - TEXT_CAPTION) / 2,
                     label, size=TEXT_CAPTION, color=draw_color)
            x += item_w

        # Crowded hint: count distinct refs that collapsed (§7)
        collapsed_refs: set[str] = set()
        for c in self._commits:
            if c["lane"] == OTHER_LANE:
                rname = c.get("_ref")
                if rname:
                    collapsed_refs.add(rname)
        if len(collapsed_refs) > 1:
            ctx.text(PAD_X, y + LEGEND_H + 2.0,
                     f"+{len(collapsed_refs)} branches collapsed into other",
                     size=TEXT_HINT, color=MUTED)

    # ── Orthogonal edge ───────────────────────────────────────────────────────

    def _draw_orthogonal_edge(self, ctx: RenderContext,
                              x1: float, y1: float,
                              x2: float, y2: float,
                              color: str) -> None:
        CUT = 8.0
        y_mid = y1 + (y2 - y1) / 2.0

        if abs(x1 - x2) < 1.0:
            ctx.line(x1, y1, x2, y2, color=color, width=2.0)
            return

        if x2 > x1:
            ctx.line(x1, y1, x1, y_mid - CUT, color=color, width=2.0)
            ctx.line(x1, y_mid - CUT, x1 + CUT, y_mid, color=color, width=2.0)
            ctx.line(x1 + CUT, y_mid, x2 - CUT, y_mid, color=color, width=2.0)
            ctx.line(x2 - CUT, y_mid, x2, y_mid + CUT, color=color, width=2.0)
            ctx.line(x2, y_mid + CUT, x2, y2, color=color, width=2.0)
        else:
            ctx.line(x1, y1, x1, y_mid - CUT, color=color, width=2.0)
            ctx.line(x1, y_mid - CUT, x1 - CUT, y_mid, color=color, width=2.0)
            ctx.line(x1 - CUT, y_mid, x2 + CUT, y_mid, color=color, width=2.0)
            ctx.line(x2 + CUT, y_mid, x2, y_mid + CUT, color=color, width=2.0)
            ctx.line(x2, y_mid + CUT, x2, y2, color=color, width=2.0)

    # ── Badge ─────────────────────────────────────────────────────────────────

    def _draw_badge(self, ctx: RenderContext, x: float, cy: float,
                    name: str, color: str) -> float:
        BADGE_PAD_H = 5.0
        BADGE_PAD_V = 3.0
        fs = TEXT_CAPTION - 1.0
        char_w = fs * 0.55
        bw = len(name) * char_w + BADGE_PAD_H * 2
        bh = fs + BADGE_PAD_V * 2

        is_tag = name.startswith("tag:")
        fill = YELLOW if is_tag else color
        radius = 2.0 if is_tag else 8.0

        ctx.rect(x, cy - bh / 2, bw, bh, fill=fill, radius=radius)
        ctx.text(x + BADGE_PAD_H, cy - fs / 2, name, size=fs, color=BG)
        return bw

    # ── Tooltip (§4) ──────────────────────────────────────────────────────────

    def _render_tooltip(self, ctx: RenderContext) -> None:
        c = self._commits[self._sel]
        # Anchor to the node's effective lane x
        effective_lane = min(c["lane"], OTHER_LANE)
        cx_node = PAD_X + effective_lane * LANE_W + LANE_W / 2
        cy_node = c.get("y", ctx.h / 2)

        TIP_W = min(TIP_W_MAX, ctx.w - 48.0)
        INNER_PAD = 12.0
        LINE_H = TEXT_BODY + 4.0

        import datetime
        dt = datetime.datetime.fromtimestamp(c["ts"]).strftime("%Y-%m-%d %H:%M")

        from plexi_sdk.ui import _wrap_to_width
        subject_lines = _wrap_to_width(
            c["subject"], TIP_W - INNER_PAD * 2, TEXT_BODY, max_lines=6
        )

        # Overflow ref names for "also:" line
        also_refs = [r["name"] for r in self._refs if r["tip_hash"] == c["hash"]][1:]

        content_h = (
            TEXT_CAPTION
            + LINE_H          # author + date
            + LINE_H * max(1, len(subject_lines))
            + LINE_H          # stats
            + (LINE_H if also_refs else 0.0)
            + INNER_PAD * 2
        )

        tip_x = cx_node + 16.0
        tip_y = cy_node - 8.0
        if tip_x + TIP_W > ctx.w - 8:
            tip_x = cx_node - TIP_W - 16.0
        if tip_y + content_h > ctx.h - 8:
            tip_y = cy_node - content_h - 8.0

        # Clamp so tooltip never renders off-pane (§4)
        tip_x = max(8.0, min(tip_x, ctx.w - TIP_W - 8.0))
        tip_y = max(8.0, min(tip_y, ctx.h - content_h - 8.0))

        ctx.rect(tip_x - 1, tip_y - 1, TIP_W + 2, content_h + 2,
                 fill=HIGHLIGHT, radius=9.0)
        ctx.rect(tip_x, tip_y, TIP_W, content_h,
                 fill=SURFACE, radius=8.0)

        tx = tip_x + INNER_PAD
        ty = tip_y + INNER_PAD

        ctx.text(tx, ty, c["short_hash"], size=TEXT_CAPTION,
                 color=ACCENT, monospace=True)
        ty += TEXT_CAPTION + 4.0

        ctx.text(tx, ty, f"{c['author']}  ·  {dt}",
                 size=TEXT_CAPTION, color=MUTED,
                 max_width=TIP_W - INNER_PAD * 2)
        ty += LINE_H

        for line in subject_lines:
            ctx.text(tx, ty, line, size=TEXT_BODY, color=FG,
                     max_width=TIP_W - INNER_PAD * 2)
            ty += LINE_H

        if not self._stats_unavailable:
            ctx.text(tx, ty, f"+{c['added']}", size=TEXT_CAPTION, color=GREEN)
            ctx.text(tx + 55.0, ty, f"-{c['removed']}", size=TEXT_CAPTION, color=RED)
        else:
            ctx.text(tx, ty, "stats unavailable", size=TEXT_CAPTION, color=MUTED)
        ty += LINE_H

        if also_refs:
            also_str = "also: " + ", ".join(also_refs)
            ctx.text(tx, ty, also_str, size=TEXT_CAPTION, color=MUTED,
                     max_width=TIP_W - INNER_PAD * 2)

    # ── Help overlay ──────────────────────────────────────────────────────────

    def _draw_help(self, ctx: RenderContext) -> None:
        help_items = [
            ("[ ]",    "go to older / newer week"),
            ("j / k",  "select next / previous commit"),
            ("esc",    "clear tooltip"),
            ("c",      "copy commit hash (status bar)"),
            ("r",      "force refresh"),
            ("?",      "toggle this help"),
            ("t",      "jump to today"),
            ("h / l",  "select commit in left / right lane"),
            ("g / G",  "first / last commit"),
            ("o",      "open on GitHub (status bar)"),
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
