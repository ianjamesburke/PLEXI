#!/usr/bin/env python3
"""gh-issues — GitHub Issues viewer for the workspace repo.

Two views:
  - LIST: scrollable issue rows with number badge, title, labels
  - DETAIL: metadata card + scrollable markdown body

Keys: j/k navigate · Enter open detail · Esc back · o open in browser
      r refresh · n new issue in terminal
"""
import asyncio
import json
import subprocess

from plexi_sdk import (
    App, RenderContext, Arg,
    theme,
    BODY, CAPTION, HINT,
    PAD, PAD_TIGHT,
)
from plexi_sdk.ui import (
    AppBar, FooterKeys, InfoTable, Section,
    Scrollable, Column, Label, Spacer,
    ListRow, RowChip, LeadingBadge,
    Component,
    TEXT_BODY,
    SPACE_MD,
    _markdown_measure_lines,
)


# ── Private markdown component ────────────────────────────────────────────────

class _MarkdownBlock(Component):
    """Embeds ctx.markdown() in the declarative component tree."""

    def __init__(self, text: str) -> None:
        self.text = text

    def measure(self, avail_w: float) -> float:
        if not self.text:
            return 0.0
        lines = _markdown_measure_lines(self.text, avail_w, TEXT_BODY, max_lines=400)
        return max(1, lines) * (TEXT_BODY + 5.0)

    def render(self, ctx, x: float, y: float, w: float, _h: float) -> None:
        if self.text:
            ctx.markdown(x, y, w, self.text)


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
        return theme.danger
    if any(w in n for w in ("p2", "enhancement", "feat")):
        return theme.warning
    if any(w in n for w in ("ready", "done", "p3", "p4")):
        return theme.success
    return theme.accent


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
        self._render_seeded  = False
        # Stable Scrollable instance — scroll offset persists across renders.
        self._body_scroll    = Scrollable(Label(""))
        ctx.status_summary("Loading…")
        self.emit.info(f"gh-issues init workspace={self._root!r}")
        self._fetch()

    async def on_render_seed(self, _ctx: RenderContext, payload: dict) -> None:
        if "_issues" in payload:
            self._render_seeded  = True
            self._issues         = payload["_issues"]
            self._loading        = payload.get("_loading", False)
            self._sel            = payload.get("_sel", 0)
            self._error          = payload.get("_error")
            self._detail         = payload.get("_detail")
            self._detail_loading = bool(payload.get("_detail_loading", False))
            requested_view       = payload.get("_view", self.VIEW_LIST)
            self._view = (
                requested_view
                if requested_view != self.VIEW_DETAIL or self._detail or self._detail_loading
                else self.VIEW_LIST
            )
            self.emit.info(f"gh-issues: seeded {len(self._issues)} issues for headless render")

    # ── data ──────────────────────────────────────────────────────────────────

    def _fetch(self) -> None:
        self._loading = True
        self._error   = None
        asyncio.get_event_loop().create_task(asyncio.to_thread(self._load_list))

    def _load_list(self) -> None:
        if getattr(self, "_render_seeded", False):
            return
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
        ctx.clear(theme.bg)

        if self._view == self.VIEW_DETAIL:
            if self._detail_loading:
                ctx.render(Column([
                    AppBar("Issues", accent=theme.accent),
                    Spacer(size=SPACE_MD),
                    Label("Loading issue…", tone="body", color=theme.muted),
                ], padding_top=0))
            elif self._error:
                ctx.render(Column([
                    AppBar("Error", accent=theme.accent),
                    Spacer(size=SPACE_MD),
                    Label(f"Error: {self._error}", tone="body", color=theme.danger),
                    Spacer(grow=True),
                    FooterKeys([("escape", "back")]),
                ], padding_top=0))
            else:
                self._draw_detail(ctx)
            return

        # Header: AppBar (title + count) + FooterKeys (shortcuts)
        appbar = AppBar(
            "Issues",
            subtitle=f"{len(self._issues)} open" if not self._loading else None,
            accent=theme.accent,
        )
        shortcuts = FooterKeys([
            (["j", "k"], "navigate"),
            ("↩", "detail"),
            ("o", "browser"),
            ("r", "refresh"),
            ("n", "new"),
        ])
        appbar_h    = appbar.measure(ctx.w)
        shortcuts_h = shortcuts.measure(ctx.w)
        appbar.render(ctx, 0.0, 0.0, ctx.w, appbar_h)
        shortcuts.render(ctx, 0.0, appbar_h, ctx.w, shortcuts_h)
        list_top = appbar_h + shortcuts_h

        if self._loading:
            ctx.text(PAD, list_top + PAD, "Loading…", size=BODY, color=theme.muted)
            return
        if self._error:
            ctx.text(PAD, list_top + PAD, f"Error: {self._error}",
                     size=CAPTION, color=theme.danger, max_width=ctx.w - PAD * 2)
            ctx.text(PAD, list_top + PAD + BODY + PAD_TIGHT,
                     "r — retry", size=HINT, color=theme.muted)
            return

        self._draw_list(ctx, list_top)

    def _draw_list(self, ctx: RenderContext, list_top: float) -> None:
        rows = [
            ListRow(
                id=f"issue-{issue['number']}",
                leading=LeadingBadge(f"#{issue['number']}", color=theme.accent),
                primary=issue.get("title", ""),
                chips=[
                    RowChip(lbl.get("name", ""), _label_color(lbl.get("name", "")))
                    for lbl in (issue.get("labels") or [])[:2]
                ],
            ).to_dict()
            for issue in self._issues
        ]
        ctx.list_view("issues", rows, selected=self._sel, y=float(list_top))

    def _draw_detail(self, ctx: RenderContext) -> None:
        if self._detail is None:
            return
        d             = self._detail
        labels_str    = ", ".join(lb.get("name", "") for lb in d.get("labels", []) if lb) or "none"
        assignees_str = ", ".join(a.get("login", "") for a in d.get("assignees", []) if a) or "unassigned"
        body_text     = (d.get("body") or "").strip()
        number        = d.get("number", "")
        title         = d.get("title", "")

        self._body_scroll.child = (
            _MarkdownBlock(body_text) if body_text
            else Label("No body.", tone="caption", color=theme.muted)
        )

        ctx.render(Column([
            AppBar(f"← #{number}  {title}", accent=theme.accent),
            InfoTable([
                ("number",    f"#{number}"),
                ("state",     d.get("state", "open")),
                ("labels",    labels_str),
                ("assignees", assignees_str),
                ("opened",    (d.get("createdAt") or "")[:10]),
            ]),
            Section("Body"),
            self._body_scroll,
            FooterKeys([("o", "open in browser"), ("escape", "back")]),
        ], padding_top=0))

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
            else:
                if self._body_scroll.handle_key(key):
                    self.emit.schedule_render()

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
        self._view                      = self.VIEW_DETAIL
        self._detail                    = None
        self._detail_loading            = True
        self._error                     = None
        self._body_scroll.scroll_offset = 0.0
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
