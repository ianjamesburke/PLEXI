#!/usr/bin/env python3
"""gh-issues — GitHub Issues viewer for the workspace repo.

Three views:
  - LIST: scrollable issue rows with number badge, title, labels
  - DETAIL: metadata card + scrollable markdown body
  - PICKER: label selector for multi-label AND filtering

Keys: j/k navigate · Enter open detail · Esc back · o open in browser
      s sort · f filter · l label picker · c clear filter · r refresh · n new
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
ISSUE_LIST_LIMIT = "500"

SORT_LABELS = {
    "created_desc": "created ↓",
    "created_asc": "created ↑",
    "number_desc": "number ↓",
    "number_asc": "number ↑",
}


PRIORITY_PREFIXES = ("p0", "p1", "p2", "p3", "p4", "bug", "enhancement", "feat", "fix")
MAX_VISIBLE_CHIPS = 3


def _issue_labels(issue: dict) -> list[str]:
    return [
        str(label.get("name", ""))
        for label in (issue.get("labels") or [])
        if label and label.get("name")
    ]


def _is_priority_label(name: str) -> bool:
    return name.lower().startswith(PRIORITY_PREFIXES)


def _select_visible_chips(
    issue: dict, active_filters: set[str],
) -> list[RowChip]:
    all_labels = _issue_labels(issue)
    if not all_labels:
        return []
    active = [l for l in all_labels if l in active_filters]
    priority = [l for l in all_labels if l not in active_filters and _is_priority_label(l)]
    rest = [l for l in all_labels if l not in active_filters and not _is_priority_label(l)]
    ordered = active + priority + rest
    visible = ordered[:MAX_VISIBLE_CHIPS]
    hidden_count = len(all_labels) - len(visible)
    chips = [RowChip(name, _label_color(name)) for name in visible]
    if hidden_count > 0:
        chips.append(RowChip(f"+{hidden_count}", theme.muted))
    return chips


def _collect_unique_labels(issues: list[dict]) -> list[str]:
    seen: set[str] = set()
    result: list[str] = []
    for issue in issues:
        for label in _issue_labels(issue):
            if label not in seen:
                seen.add(label)
                result.append(label)
    result.sort(key=str.lower)
    return result


def _fuzzy_match(query: str, label: str) -> bool:
    return query.lower() in label.lower()


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
    filter_labels: set[str],
    sort_mode: str,
) -> list[dict]:
    visible = list(issues)
    if filter_labels:
        visible = [
            issue for issue in visible
            if filter_labels <= set(_issue_labels(issue))
        ]
    reverse = sort_mode in ("created_desc", "number_desc")
    return sorted(visible, key=lambda issue: _sort_key(issue, sort_mode), reverse=reverse)


# ── App ───────────────────────────────────────────────────────────────────────

class GhIssues(App):
    VIEW_LIST   = "list"
    VIEW_DETAIL = "detail"
    VIEW_PICKER = "picker"

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
        self._filter_labels  : set[str] = set()
        self._sort_mode      = "created_desc"
        self._body_scroll    = Scrollable(Label(""))
        self._picker_query   = ""
        self._picker_sel     = 0
        self._picker_staged  : set[str] = set()
        self.emit.status_summary("Loading…")
        self.emit.info(f"gh-issues init workspace={self._root!r}")
        self._fetch()

    # ── data ──────────────────────────────────────────────────────────────────

    def _fetch(self) -> None:
        self._loading       = True
        self._error         = None
        self._filter_labels = set()
        asyncio.create_task(asyncio.to_thread(self._load_list))

    def _load_list(self) -> None:
        rc, out, err = _gh(
            "issue", "list", "--state", "open",
            "--json", "number,title,state,labels,assignees,createdAt",
            "--limit", ISSUE_LIST_LIMIT,
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
        return _filter_and_sort_issues(self._issues, self._filter_labels, self._sort_mode)

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
        if self._filter_labels:
            label_str = "+".join(sorted(self._filter_labels))
            parts.append(f"label:{label_str}")
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

        if self._view == self.VIEW_PICKER:
            self._draw_picker(ctx)
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
            ("l", "labels"),
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
                chips=_select_visible_chips(issue, self._filter_labels),
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

    def _picker_filtered_labels(self) -> list[str]:
        all_labels = _collect_unique_labels(self._issues)
        if not self._picker_query:
            return all_labels
        return [l for l in all_labels if _fuzzy_match(self._picker_query, l)]

    def _draw_picker(self, ctx: RenderContext) -> None:
        filtered = self._picker_filtered_labels()
        self._picker_sel = max(0, min(self._picker_sel, len(filtered) - 1)) if filtered else 0

        query_display = self._picker_query or ""
        subtitle = f"{len(self._picker_staged)} selected" if self._picker_staged else "type to filter"
        if query_display:
            subtitle = f'"{query_display}" · {subtitle}'

        appbar = AppBar("Labels", subtitle=subtitle, accent=theme.accent)
        footer = FooterKeys([
            ("↩", "apply"),
            ("space", "toggle"),
            ("escape", "cancel"),
        ])
        appbar_h = appbar.measure(ctx.w)
        footer_h = footer.measure(ctx.w)
        appbar.render(ctx, 0.0, 0.0, ctx.w, appbar_h)
        footer.render(ctx, 0.0, ctx.h - footer_h, ctx.w, footer_h)

        rows = [
            ListRow(
                id=f"label-{i}",
                leading=LeadingBadge("✓" if label in self._picker_staged else " ", color=theme.accent if label in self._picker_staged else theme.muted),
                primary=label,
                chips=[RowChip(label, _label_color(label))],
            ).to_dict()
            for i, label in enumerate(filtered)
        ]
        list_h = max(0.0, ctx.h - appbar_h - footer_h)
        if rows:
            ctx.list_view("label-picker", rows, selected=self._picker_sel,
                          y=float(appbar_h), h=float(list_h))
        else:
            ctx.text(PAD, appbar_h + PAD, "No matching labels.", size=BODY, color=theme.muted)

    # ── input ─────────────────────────────────────────────────────────────────

    def on_escape(self) -> bool:
        if self._view == self.VIEW_PICKER:
            self._view = self.VIEW_LIST
            self.emit.info("gh-issues: picker cancelled")
            self.emit.schedule_render()
            return True
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

        if self._view == self.VIEW_PICKER:
            self._handle_picker_key(key)
            return

        if self._view == self.VIEW_LIST:
            if key == "o":
                await self._open_browser()
            elif key == "s":
                self._cycle_sort()
            elif key == "f":
                self._toggle_filter_from_selection()
            elif key == "l":
                self._open_picker()
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

    def on_list_select(self, list_id: str, index: int) -> None:
        if list_id == "label-picker":
            self._picker_sel = index
            self.emit.schedule_render()
            return
        self._sel = index
        issue = self._selected_issue()
        if issue:
            self.emit.status_summary(issue["title"])
        self.emit.schedule_render()

    def on_list_activate(self, list_id: str, _index: int) -> None:
        if list_id == "label-picker":
            self._apply_picker()
            return
        self._open_detail()

    def on_click(self, _x: float, _y: float, _button: str) -> None:
        pass

    # ── actions ───────────────────────────────────────────────────────────────

    def _cycle_sort(self) -> None:
        self._sort_mode = _next_sort_mode(self._sort_mode)
        self._clamp_selection()
        self.emit.info(f"gh-issues: sort {SORT_LABELS[self._sort_mode]}")
        self.emit.schedule_render()

    def _toggle_filter_from_selection(self) -> None:
        issue = self._selected_issue()
        if not issue:
            return
        labels = _issue_labels(issue)
        if not labels:
            self.emit.info("gh-issues: selected issue has no labels to filter")
            return
        keep_number = issue.get("number")
        current = next(iter(self._filter_labels), None) if len(self._filter_labels) == 1 else None
        if current in labels:
            idx = labels.index(current)
            if idx == len(labels) - 1:
                self._filter_labels = set()
                self._select_issue_number(keep_number)
                self.emit.info(f"gh-issues: cleared filter label:{current}")
                self.emit.schedule_render()
                return
            self._filter_labels = {labels[idx + 1]}
        else:
            self._filter_labels = {labels[0]}
        self._select_issue_number(keep_number)
        label_str = next(iter(self._filter_labels))
        self.emit.info(f"gh-issues: filter label:{label_str}")
        self.emit.schedule_render()

    def _clear_filter(self) -> None:
        if not self._filter_labels:
            return
        issue = self._selected_issue()
        keep_number = issue.get("number") if issue else None
        cleared = "+".join(sorted(self._filter_labels))
        self._filter_labels = set()
        self._select_issue_number(keep_number)
        self.emit.info(f"gh-issues: cleared filter label:{cleared}")
        self.emit.schedule_render()

    def _open_picker(self) -> None:
        self._view = self.VIEW_PICKER
        self._picker_query = ""
        self._picker_sel = 0
        self._picker_staged = set(self._filter_labels)
        self.emit.info("gh-issues: label picker opened")
        self.emit.schedule_render()

    def _apply_picker(self) -> None:
        self._filter_labels = set(self._picker_staged)
        self._view = self.VIEW_LIST
        self._clamp_selection()
        label_str = "+".join(sorted(self._filter_labels)) if self._filter_labels else "none"
        self.emit.info(f"gh-issues: picker applied labels:{label_str}")
        self.emit.schedule_render()

    def _handle_picker_key(self, key: str) -> None:
        filtered = self._picker_filtered_labels()
        if key == " ":
            if filtered and 0 <= self._picker_sel < len(filtered):
                label = filtered[self._picker_sel]
                if label in self._picker_staged:
                    self._picker_staged.discard(label)
                else:
                    self._picker_staged.add(label)
            self.emit.schedule_render()
        elif key == "Backspace":
            if self._picker_query:
                self._picker_query = self._picker_query[:-1]
                self._picker_sel = 0
                self.emit.schedule_render()
        elif len(key) == 1 and key.isprintable():
            self._picker_query += key
            self._picker_sel = 0
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
