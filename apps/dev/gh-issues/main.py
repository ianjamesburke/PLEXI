#!/usr/bin/env python3
"""gh-issues — GitHub Issues viewer for the workspace repo.

Two views:
  - LIST: scrollable issue rows with number badge, title, labels
  - DETAIL: terminal-style metadata table + markdown body

Keys: j/k navigate · Enter open detail · Esc back · o open in browser
      r refresh · n new issue in terminal
"""
import asyncio
import json
import subprocess

from plexi_sdk import (
    App, RenderContext, Arg,
    BG, SURFACE, FG, ACCENT, MUTED, HIGHLIGHT,
    BODY, CAPTION, HEADING, HINT,
    GREEN, RED, YELLOW,
    PAD, PAD_TIGHT,
)
from plexi_sdk.ui import ListRow, RowChip, LeadingBadge

# ── Layout ────────────────────────────────────────────────────────────────────

HEADER_H = 52
ROW_H    = 38
KEY_COL  = 90


# ── Helpers ───────────────────────────────────────────────────────────────────

def _gh(*args: str, cwd: str | None = None, timeout: float = 15.0) -> tuple[int, str, str]:
    """Run gh CLI. Returns (returncode, stdout, stderr). Never raises."""
    try:
        p = subprocess.run(
            ["gh", *args], cwd=cwd, capture_output=True, text=True, timeout=timeout,
        )
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return -1, "", "timeout after 15s"
    except Exception as exc:
        return -1, "", str(exc)


def _label_color(name: str) -> str:
    n = name.lower()
    if any(w in n for w in ("bug", "error", "p0", "p1")):
        return RED
    if any(w in n for w in ("p2", "enhancement", "feat")):
        return YELLOW
    if any(w in n for w in ("ready", "done", "p3", "p4")):
        return GREEN
    return ACCENT


# ── App ───────────────────────────────────────────────────────────────────────

class GhIssues(App):
    VIEW_LIST   = "list"
    VIEW_DETAIL = "detail"

    repo_dir: Arg[str | None] = Arg("--repo-dir", default=lambda ctx: ctx.workspace_root)

    async def on_init(self, ctx: RenderContext) -> None:
        self._view           = self.VIEW_LIST
        self._issues         : list[dict] = []
        self._sel            = 0
        self._loading        = True
        self._detail_loading = False
        self._error          : str | None = None
        self._detail         : dict | None = None
        self._root           = self.repo_dir or ""
        ctx.status_summary("Loading…")
        self.emit.info(f"gh-issues init workspace={self._root!r}")
        self._fetch()

    # ── data ──────────────────────────────────────────────────────────────────

    def _fetch(self) -> None:
        self._loading = True
        self._error   = None
        asyncio.get_event_loop().create_task(asyncio.to_thread(self._load_list))

    def _load_list(self) -> None:
        rc, out, err = _gh(
            "issue", "list", "--state", "open",
            "--json", "number,title,state,labels,assignees,createdAt",
            "--limit", "100",
            cwd=self._root or None,
        )
        if rc != 0:
            self.emit.warn(f"gh issue list failed rc={rc} stderr={err!r}")
            self._error   = err.strip() or f"exit {rc}"
            self._loading = False
            self.emit.schedule_render()
            return
        try:
            self._issues = json.loads(out)
            # clamp selection to new list bounds
            self._sel = max(0, min(self._sel, len(self._issues) - 1)) if self._issues else 0
        except Exception as exc:
            self.emit.warn(f"gh issue list json parse error: {exc}")
            self._error = str(exc)
        self._loading = False
        self.emit.info(f"gh-issues loaded {len(self._issues)} open issues")
        self.emit.schedule_render()

    def _load_detail(self, number: int) -> None:
        rc, out, err = _gh(
            "issue", "view", str(number),
            "--json", "number,title,state,body,labels,assignees,createdAt",
            cwd=self._root or None,
        )
        if rc != 0:
            self.emit.warn(f"gh issue view {number} failed rc={rc} stderr={err!r}")
            self._error = err.strip() or f"exit {rc}"
            self._detail_loading = False
            self.emit.schedule_render()
            return
        try:
            self._detail = json.loads(out)
            self.emit.info(f"gh-issues loaded detail #{number}")
        except Exception as exc:
            self.emit.warn(f"gh issue view {number} json parse error: {exc}")
            self._error = str(exc)
        self._detail_loading = False
        self.emit.schedule_render()

    # ── render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)
        self._draw_header(ctx)

        if self._loading:
            ctx.text(PAD, HEADER_H + PAD, "Loading…", size=BODY, color=MUTED)
            return
        if self._error:
            ctx.text(PAD, HEADER_H + PAD, f"Error: {self._error}",
                     size=CAPTION, color=RED, max_width=ctx.w - PAD * 2)
            ctx.text(PAD, HEADER_H + PAD + BODY + PAD_TIGHT,
                     "r — retry", size=HINT, color=MUTED)
            return

        if self._view == self.VIEW_DETAIL:
            if self._detail_loading:
                ctx.text(PAD, HEADER_H + PAD, "Loading issue…", size=BODY, color=MUTED)
                return
            self._draw_detail(ctx)
        else:
            self._draw_list(ctx)

    def _draw_header(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, HEADER_H, fill=SURFACE)
        ctx.rect(0, HEADER_H - 1, ctx.w, 1, fill=BG)

        mid_y = HEADER_H / 2
        ctx.text(PAD, mid_y - HEADING / 2, "Issues", size=HEADING, color=ACCENT,
                 bold=True, monospace=True)

        if not self._loading:
            ctx.badge(
                x=PAD + 76, y_center=mid_y,
                label=f"{len(self._issues)} open",
                fill=SURFACE, fg=MUTED, font_size=HINT, radius=3.0,
            )

        ctx.shortcuts(
            x=ctx.w / 2, y=mid_y - HINT / 2,
            max_width=ctx.w / 2 - PAD,
            pairs=[
                (["j", "k"], "navigate"),
                ("↩", "detail"),
                ("o", "browser"),
                ("r", "refresh"),
                ("n", "new"),
            ],
        )

    def _draw_list(self, ctx: RenderContext) -> None:
        rows = [
            ListRow(
                id=f"issue-{issue['number']}",
                leading=LeadingBadge(f"#{issue['number']}", color="accent"),
                primary=issue.get("title", ""),
                chips=[
                    RowChip(lbl.get("name", ""), _label_color(lbl.get("name", "")))
                    for lbl in (issue.get("labels") or [])[:2]
                ],
            ).to_dict()
            for issue in self._issues
        ]
        ctx.list_view(
            "issues",
            rows,
            selected=self._sel,
            loading=False,
            y=float(HEADER_H),
        )

    def _draw_detail(self, ctx: RenderContext) -> None:
        if self._detail is None:
            return
        d  = self._detail
        y  = float(HEADER_H + PAD)

        ctx.text(PAD, y, "← esc", size=HINT, color=MUTED)
        y += HINT + PAD

        # title
        ctx.text(PAD, y, d["title"], size=HEADING, color=FG,
                 bold=True, max_width=ctx.w - PAD * 2)
        y += HEADING + PAD

        # metadata table (1073 style: SURFACE bg, GREEN keys, FG values)
        labels_str    = ", ".join(lb["name"] for lb in d.get("labels", [])) or "none"
        assignees_str = ", ".join(a["login"] for a in d.get("assignees", [])) or "unassigned"
        rows = [
            ("number",    f"#{d['number']}"),
            ("state",     d.get("state", "open")),
            ("labels",    labels_str),
            ("assignees", assignees_str),
            ("opened",    (d.get("createdAt") or "")[:10]),
        ]
        T_ROW_H = 28
        table_h = T_ROW_H * len(rows)
        ctx.rect(PAD, y, ctx.w - PAD * 2, table_h, fill=SURFACE, radius=6.0)
        for i, (key, value) in enumerate(rows):
            ry = y + i * T_ROW_H
            if i > 0:
                ctx.rect(PAD, ry, ctx.w - PAD * 2, 1, fill=BG)
            ty = ry + (T_ROW_H - CAPTION) / 2
            ctx.text(PAD + 10, ty, key,   size=CAPTION, color=GREEN, monospace=True)
            ctx.text(PAD + 10 + KEY_COL, ty, value, size=CAPTION, color=FG,
                     monospace=True, max_width=ctx.w - PAD - 10 - KEY_COL - PAD)
        y += table_h + PAD

        # body (markdown)
        body = (d.get("body") or "").strip()
        if body:
            ctx.text(PAD, y, "body", size=CAPTION, color=MUTED, monospace=True)
            y += CAPTION + PAD_TIGHT
            ctx.markdown(PAD, y, ctx.w - PAD * 2, body)

        # footer shortcuts
        ctx.shortcuts(
            x=PAD, y=ctx.h - PAD - HINT,
            max_width=ctx.w - PAD * 2,
            pairs=[("o", "open in browser"), ("esc", "back")],
        )

    # ── input ─────────────────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, _mods: dict) -> None:
        if self._loading:
            return

        if self._view == self.VIEW_LIST:
            if key == "o":
                self._open_browser()
            elif key == "r":
                self.emit.info("gh-issues: refresh")
                self._fetch()
            elif key == "n":
                self._new_issue()

        elif self._view == self.VIEW_DETAIL:
            if key == "escape":
                self._view   = self.VIEW_LIST
                self._detail = None
                self.emit.info("gh-issues: back to list")
                ctx.status_summary("Issues")
            elif key == "o":
                if self._detail:
                    num = self._detail["number"]
                    rc, _, _ = _gh("issue", "view", str(num), "--web",
                                   cwd=self._root or None)
                    self.emit.info(f"gh-issues: open #{num} in browser rc={rc}")

    def on_list_select(self, ctx: RenderContext, _id: str, index: int) -> None:
        self._sel = index
        if self._issues:
            ctx.status_summary(self._issues[self._sel]["title"])
        self.emit.schedule_render()

    def on_list_activate(self, _ctx: RenderContext, _id: str, _index: int) -> None:
        self._open_detail()

    def on_click(self, _ctx: RenderContext, _x: float, _y: float, _button: str) -> None:
        pass

    # ── actions ───────────────────────────────────────────────────────────────

    def _open_detail(self) -> None:
        if not self._issues:
            return
        issue = self._issues[self._sel]
        self.emit.info(f"gh-issues: open detail #{issue['number']}")
        self._view           = self.VIEW_DETAIL
        self._detail         = None
        self._detail_loading = True
        asyncio.get_event_loop().create_task(
            asyncio.to_thread(self._load_detail, issue["number"])
        )

    def _open_browser(self) -> None:
        if not self._issues:
            return
        num = self._issues[self._sel]["number"]
        rc, _, _ = _gh("issue", "view", str(num), "--web", cwd=self._root or None)
        self.emit.info(f"gh-issues: open #{num} in browser rc={rc}")

    def _new_issue(self) -> None:
        self.emit.info("gh-issues: new issue")
        self.emit.run_in_terminal("gh issue create")


GhIssues().run()
