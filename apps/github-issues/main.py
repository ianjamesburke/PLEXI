#!/usr/bin/env python3
"""gh-issues — GitHub Issues viewer for the workspace repo.

Two views:
  - LIST: scrollable issue rows with number badge, title, labels
  - DETAIL: metadata card + scrollable markdown body

Keys: j/k navigate · Enter open detail · Esc back · o open in browser
      s sort · f filter · c clear filter · r refresh · n new issue in terminal
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


SORT_MODES = ("created_desc", "created_asc", "number_desc", "number_asc")

SORT_LABELS = {
    "created_desc": "created ↓",
    "created_asc": "created ↑",
    "number_desc": "number ↓",
    "number_asc": "number ↑",
}


def _issue_labels(issue: dict) -> list[str]:
    return [
        str(label.get("name", ""))
        for label in (issue.get("labels") or [])
        if label and label.get("name")
    ]


def _next_sort_mode(mode: str) -> str:
    try:
        idx = SORT_MODES.index(mode)
    except ValueError:
        idx = 0
    return SORT_MODES[(idx + 1) % len(SORT_MODES)]


def _sort_key(issue: dict, mode: str) -> tuple:
    if mode.startswith("number"):
        return (int(issue.get("number") or 0),)
    return (str(issue.get("createdAt") or ""), int(issue.get("number") or 0))


def _filter_and_sort_issues(
    issues: list[dict],
    filter_label: str | None,
    sort_mode: str,
) -> list[dict]:
    visible = list(issues)
    if filter_label:
        visible = [
            issue for issue in visible
            if any(label == filter_label for label in _issue_labels(issue))
        ]
    reverse = sort_mode in ("created_desc", "number_desc")
    return sorted(visible, key=lambda issue: _sort_key(issue, sort_mode), reverse=reverse)


# ── App ───────────────────────────────────────────────────────────────────────

class GhIssues(App):
    VIEW_LIST   = "list"
    VIEW_DETAIL = "detail"

    repo_dir: Arg[str | None] = Arg("--repo-dir", default=lambda ctx: ctx.workspace_root)

    async def on_init(self) -> None:
        self._view           = self.VIEW_LIST
        self._issues         : list[dict] = []
        self._sel            = 0
        self._loading        = True
        self._detail_loading = False
        self._error          : str | None = None
        self._detail         : dict | None = None
        self._root           = self.repo_dir or ""
        self._filter_label   : str | None = None
        self._sort_mode      = "created_desc"
        # Stable Scrollable instance — scroll offset persists across renders.
        self._body_scroll    = Scrollable(Label(""))
        self.emit.status_summary("Loading…")
        self.emit.info(f"gh-issues init workspace={self._root!r}")
        self._fetch()

    # ── data ──────────────────────────────────────────────────────────────────

    def _fetch(self) -> None:
        self._loading      = True
        self._error        = None
        self._filter_label = None
        asyncio.create_task(asyncio.to_thread(self._load_list))

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
            self._clamp_selection()
        except Exception as exc:
            self.emit.warn(f"gh issue list json parse error: {exc}")
            self._error = str(exc)
        self._loading = False
        self.emit.info(f"gh-issues loaded {len(self._issues)} open issues")
        self.emit.schedule_render()

    def _visible_issues(self) -> list[dict]:
        return _filter_and_sort_issues(self._issues, self._filter_label, self._sort_mode)

    def _clamp_selection(self) -> None:
        visible = self._visible_issues()
        self._sel = max(0, min(self._sel, len(visible) - 1)) if visible else 0

    def _selected_issue(self) -> dict | None:
        visible = self._visible_issues()
        if not visible:
            return None
        self._sel = max(0, min(self._sel, len(visible) - 1))
        return visible[self._sel]

    def _select_issue_number(self, number: int | None) -> None:
        visible = self._visible_issues()
        if number is not None:
            for idx, issue in enumerate(visible):
                if issue.get("number") == number:
                    self._sel = idx
                    return
        self._clamp_selection()

    def _list_subtitle(self) -> str:
        count = len(self._visible_issues())
        parts = [f"{count} open"]
        if self._filter_label:
            parts.append(f"label:{self._filter_label}")
        parts.append(SORT_LABELS.get(self._sort_mode, SORT_LABELS["created_desc"]))
        return " · ".join(parts)

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

        # No-repo message when context root has no GitHub repo.
        if not self._root:
            ctx.render(Column([
                AppBar("GitHub Issues", accent=theme.accent),
                Spacer(size=SPACE_MD),
                Label(
                    "Set the context root to a directory with a GitHub repo in order to see issues.",
                    tone="body",
                    color=theme.muted,
                ),
            ], padding_top=0))
            return

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

        # Header: AppBar (title + count). Shortcuts live in a bottom footer.
        appbar = AppBar(
            "Issues",
            subtitle=self._list_subtitle() if not self._loading else None,
            accent=theme.accent,
        )
        footer = FooterKeys([
            ("↩", "detail"),
            ("s", "sort"),
            ("f", "filter"),
            ("c", "clear"),
            ("o", "browser"),
            ("r", "refresh"),
            ("n", "new"),
        ])
        appbar_h  = appbar.measure(ctx.w)
        footer_h  = footer.measure(ctx.w)
        list_top  = appbar_h
        appbar.render(ctx, 0.0, 0.0, ctx.w, appbar_h)
        footer.render(ctx, 0.0, ctx.h - footer_h, ctx.w, footer_h)

        if self._loading:
            ctx.text(PAD, list_top + PAD, "Loading…", size=BODY, color=theme.muted)
            return
        if self._error:
            ctx.text(PAD, list_top + PAD, f"Error: {self._error}",
                     size=CAPTION, color=theme.danger, max_width=ctx.w - PAD * 2)
            ctx.text(PAD, list_top + PAD + BODY + PAD_TIGHT,
                     "r — retry", size=HINT, color=theme.muted)
            return

        self._draw_list(ctx, list_top, footer_h)

    def _draw_list(self, ctx: RenderContext, list_top: float, footer_h: float) -> None:
        visible = self._visible_issues()
        self._clamp_selection()
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
            for issue in visible
        ]
        list_h = max(0.0, ctx.h - list_top - footer_h)
        ctx.list_view("issues", rows, selected=self._sel,
                      y=float(list_top), h=float(list_h))

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

    def on_escape(self) -> bool:
        if self._view == self.VIEW_DETAIL:
            self._view   = self.VIEW_LIST
            self._detail = None
            self._error  = None
            self.emit.info("gh-issues: back to list")
            self.emit.status_summary("Issues")
            self.emit.schedule_render()
            return True
        return False

    async def on_key(self, key: str, _mods: dict) -> None:
        if self._loading:
            return

        if self._view == self.VIEW_LIST:
            if key == "o":
                await self._open_browser()
            elif key == "s":
                self._cycle_sort()
            elif key == "f":
                self._toggle_filter_from_selection()
            elif key == "c":
                self._clear_filter()
            elif key == "r":
                self.emit.info("gh-issues: refresh")
                self._fetch()
            elif key == "n":
                self._new_issue()

        elif self._view == self.VIEW_DETAIL:
            if key == "o":
                if self._detail:
                    num = self._detail["number"]
                    rc, _, _ = await asyncio.to_thread(
                        _gh, "issue", "view", str(num), "--web", cwd=self._root or None,
                    )
                    self.emit.info(f"gh-issues: open #{num} in browser rc={rc}")
            else:
                if self._body_scroll.handle_key(key):
                    self.emit.schedule_render()

    def on_list_select(self, _id: str, index: int) -> None:
        self._sel = index
        issue = self._selected_issue()
        if issue:
            self.emit.status_summary(issue["title"])
        self.emit.schedule_render()

    def on_list_activate(self, _id: str, _index: int) -> None:
        self._open_detail()

    def on_click(self, _x: float, _y: float, _button: str) -> None:
        pass

    # ── actions ───────────────────────────────────────────────────────────────

    def _cycle_sort(self) -> None:
        issue = self._selected_issue()
        keep_number = issue.get("number") if issue else None
        self._sort_mode = _next_sort_mode(self._sort_mode)
        self._select_issue_number(keep_number)
        self.emit.info(f"gh-issues: sort {SORT_LABELS[self._sort_mode]}")
        self.emit.schedule_render()

    def _toggle_filter_from_selection(self) -> None:
        if self._filter_label:
            self._clear_filter()
            return
        issue = self._selected_issue()
        if not issue:
            return
        labels = _issue_labels(issue)
        if not labels:
            self.emit.info("gh-issues: selected issue has no labels to filter")
            return
        self._filter_label = labels[0]
        self._select_issue_number(issue.get("number"))
        self.emit.info(f"gh-issues: filter label:{self._filter_label}")
        self.emit.schedule_render()

    def _clear_filter(self) -> None:
        if not self._filter_label:
            return
        issue = self._selected_issue()
        keep_number = issue.get("number") if issue else None
        cleared = self._filter_label
        self._filter_label = None
        self._select_issue_number(keep_number)
        self.emit.info(f"gh-issues: cleared filter label:{cleared}")
        self.emit.schedule_render()

    def _open_detail(self) -> None:
        issue = self._selected_issue()
        if not issue:
            return
        self.emit.info(f"gh-issues: open detail #{issue['number']}")
        self._view                      = self.VIEW_DETAIL
        self._detail                    = None
        self._detail_loading            = True
        self._error                     = None
        self._body_scroll.scroll_offset = 0.0
        asyncio.create_task(
            asyncio.to_thread(self._load_detail, issue["number"])
        )

    async def _open_browser(self) -> None:
        issue = self._selected_issue()
        if not issue:
            return
        num = issue["number"]
        rc, _, _ = await asyncio.to_thread(
            _gh, "issue", "view", str(num), "--web", cwd=self._root or None,
        )
        self.emit.info(f"gh-issues: open #{num} in browser rc={rc}")

    def _new_issue(self) -> None:
        self.emit.info("gh-issues: new issue")
        self.emit.run_in_terminal("gh issue create")


if __name__ == "__main__":
    GhIssues().run()
