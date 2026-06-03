#!/usr/bin/env python3
"""gh-projects — read-only GitHub Projects Kanban board."""
from __future__ import annotations
import asyncio
import json
import subprocess
import sys
from dataclasses import dataclass, field
from typing import Literal, cast

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    AppBar, Column, Component, FooterKeys, Label, SelectList, Spacer,
    SPACE_SM,
    TEXT_CAPTION, TEXT_BODY,
)

# ── Palette ───────────────────────────────────────────────────────────────────
COL_ACTIVE = "#18182a"
COL_BG     = "#14141c"
DIVIDER    = "#252535"
CARD_BG    = "#1c1c28"
CARD_FOCUS = "#21213a"
CARD_SEL   = "#282850"
TEXT       = "#dcdcf0"
TEXT_MID   = "#8888a8"
TEXT_DIM   = "#505068"
ACCENT     = "#7b9ef0"
SEL_BAR    = "#5068d8"
ERROR_COL  = "#d04050"

LABEL_FILLS = ["#2e3060", "#1e3a70", "#1a4a36", "#3a2060", "#603020"]
LABEL_TEXT  = "#aab0e0"

BADGE_FILLS = ["#2e3060", "#1e3a70", "#1a4a36"]
BADGE_TEXT  = "#aab0e0"

# ── Layout constants ──────────────────────────────────────────────────────────
COL_GAP  = 8
COL_PAD  = 10
COL_R    = 10.0
CARD_H   = 72
CARD_GAP = 5
CARD_X   = 10
CARD_Y   = 44


# ── Data model ────────────────────────────────────────────────────────────────

@dataclass
class Item:
    item_id: str
    title:   str
    number:  int | None
    itype:   str          # "Issue" | "PullRequest" | "DraftIssue"
    labels:  list[str]
    url:     str


@dataclass
class KanbanColumn:
    name:  str
    items: list[Item] = field(default_factory=list)


@dataclass
class Project:
    number: int
    title:  str
    owner:  str


# ── Async helpers ─────────────────────────────────────────────────────────────

def _run_gh(*args: str, cwd: str | None = None) -> "dict[str, object] | None":
    """Run a gh command synchronously; return parsed JSON dict or None on error."""
    try:
        result = subprocess.run(
            ["gh", *args],
            capture_output=True, text=True, cwd=cwd, timeout=15,
        )
        if result.returncode != 0:
            return None
        parsed = json.loads(result.stdout)
        return parsed if isinstance(parsed, dict) else {"_list": parsed}
    except Exception:
        return None


def _run_graphql(query: str, variables: dict[str, object] | None = None) -> dict[str, object] | None:
    """Run a gh api graphql query; return parsed data dict or None on error."""
    cmd: list[str] = ["gh", "api", "graphql", "-f", f"query={query}"]
    for k, v in (variables or {}).items():
        cmd.extend(["-F" if isinstance(v, (int, float, bool)) else "-f", f"{k}={v}"])
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
        if result.returncode != 0:
            return None
        parsed = json.loads(result.stdout)
        return cast(dict[str, object], parsed.get("data")) if isinstance(parsed, dict) else None
    except Exception:
        return None


_FIELDS_QUERY = """
query($owner: String!, $number: Int!) {
  user(login: $owner) {
    projectV2(number: $number) {
      fields(first: 30) {
        nodes {
          ... on ProjectV2SingleSelectField {
            name
            options { name }
          }
        }
      }
    }
  }
}"""

_ITEMS_QUERY = """
query($owner: String!, $number: Int!, $cursor: String) {
  user(login: $owner) {
    projectV2(number: $number) {
      items(first: 100, after: $cursor) {
        pageInfo { hasNextPage endCursor }
        nodes {
          id
          fieldValueByName(name: "Status") {
            ... on ProjectV2ItemFieldSingleSelectValue { name }
          }
          content {
            ... on Issue { number title url labels(first: 10) { nodes { name } } }
            ... on PullRequest { number title url labels(first: 10) { nodes { name } } }
            ... on DraftIssue { title }
          }
        }
      }
    }
  }
}"""


def _fetch_board(project: Project) -> tuple[list[KanbanColumn], str]:
    """
    Return (columns, error_msg). columns is empty on error.
    Uses raw GraphQL — gh project CLI subcommands are broken in gh ≥2.92.
    """
    data = _run_graphql(_FIELDS_QUERY, {"owner": project.owner, "number": project.number})
    if not data:
        return [], "Failed to fetch project fields"

    proj = cast(dict[str, object], (cast(dict[str, object], data.get("user") or {}).get("projectV2") or {}))
    fields_nodes = cast(list[dict[str, object]], cast(dict[str, object], proj.get("fields") or {}).get("nodes") or [])

    status_options: list[str] = []
    for f in fields_nodes:
        if f.get("name") == "Status" and f.get("options"):
            status_options = [str(o.get("name", "")) for o in cast(list[dict[str, object]], f["options"]) if o.get("name")]
            break

    if not status_options:
        return [], "No Status field found on project"

    col_map: dict[str, list[Item]] = {name: [] for name in status_options}

    cursor: str | None = None
    for _ in range(10):
        variables: dict[str, object] = {"owner": project.owner, "number": project.number}
        if cursor:
            variables["cursor"] = cursor
        data = _run_graphql(_ITEMS_QUERY, variables)
        if not data:
            return [], "Failed to fetch project items"

        proj = cast(dict[str, object], (cast(dict[str, object], data.get("user") or {}).get("projectV2") or {}))
        items_obj = cast(dict[str, object], proj.get("items") or {})
        page_info = cast(dict[str, object], items_obj.get("pageInfo") or {})
        nodes = cast(list[dict[str, object]], items_obj.get("nodes") or [])

        for node in nodes:
            content = cast(dict[str, object], node.get("content") or {})
            status_field = cast(dict[str, object], node.get("fieldValueByName") or {})
            status = str(status_field.get("name") or "").strip()

            number_raw = content.get("number")
            try:
                number = int(number_raw) if number_raw is not None else None  # type: ignore[arg-type]
            except (ValueError, TypeError):
                number = None

            itype = "DraftIssue" if number is None else "Issue"
            title = str(content.get("title") or "(no title)")
            url = str(content.get("url") or "")
            labels: list[str] = []
            for lbl in cast(list[dict[str, object]], cast(dict[str, object], content.get("labels") or {}).get("nodes") or []):
                lname = str(lbl.get("name", ""))
                if lname:
                    labels.append(lname)

            item = Item(
                item_id=str(node.get("id") or ""),
                title=title,
                number=number,
                itype=itype,
                labels=labels,
                url=url,
            )
            if status in col_map:
                col_map[status].append(item)

        if not page_info.get("hasNextPage"):
            break
        cursor = str(page_info.get("endCursor") or "")

    columns = [KanbanColumn(name=name, items=col_map.get(name, [])) for name in status_options]
    return columns, ""


_PROJECTS_QUERY = """
query($owner: String!) {
  user(login: $owner) {
    projectsV2(first: 50) {
      nodes { number title }
    }
  }
}"""


def _fetch_projects(owner: str) -> list[Project]:
    data = _run_graphql(_PROJECTS_QUERY, {"owner": owner})
    if not data:
        return []
    nodes = cast(list[dict[str, object]],
                 cast(dict[str, object],
                      cast(dict[str, object], data.get("user") or {}).get("projectsV2") or {}).get("nodes") or [])
    return [
        Project(number=int(str(p.get("number") or 0)), title=str(p.get("title") or ""), owner=owner)
        for p in nodes
    ]


def _detect_repo(workspace_root: str) -> tuple[str, str]:
    """Return (owner, name) or ("", "") on failure."""
    data = _run_gh("repo", "view", "--json", "owner,name", cwd=workspace_root or None)
    if not data:
        return "", ""
    owner_raw = data.get("owner")
    owner = (cast(dict[str, object], owner_raw).get("login") or "") if isinstance(owner_raw, dict) else str(owner_raw or "")
    name = str(data.get("name") or "")
    return str(owner), name


def _open_url(url: str) -> None:
    try:
        subprocess.run(["open", url], timeout=5)
    except Exception:
        pass


# ── Custom kanban body component ──────────────────────────────────────────────

class KanbanBody(Component):
    """Multi-column kanban board. Renders via raw draws; no SDK horizontal container exists."""

    def __init__(self, columns: list[KanbanColumn], focused_col: int, focused_card: int) -> None:
        self._columns = columns
        self._focused_col = focused_col
        self._focused_card = focused_card

    def is_grow(self) -> bool:
        return True

    def measure(self, _avail_w: float) -> float:
        return 0.0  # grow — allocated by parent

    def _col_rect(self, w: float, h: float, i: int) -> tuple[float, float, float, float]:
        n = max(1, len(self._columns))
        cw = (w - COL_PAD * 2 - COL_GAP * (n - 1)) / n
        cx = COL_PAD + i * (cw + COL_GAP)
        return cx, 0.0, cw, h

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        for i, col in enumerate(self._columns):
            cx, _, cw, ch = self._col_rect(w, h, i)
            focused = (i == self._focused_col)
            ax = x + cx
            ay = y

            ctx.rect(ax, ay, cw, ch, fill=COL_ACTIVE if focused else COL_BG, radius=COL_R)

            # column header band
            hdr_bg = "#1e1e38" if focused else "#161622"
            ctx.rect(ax, ay, cw, 36, fill=hdr_bg, radius=COL_R)
            ctx.rect(ax, ay + 28, cw, 8, fill=hdr_bg)
            ctx.rect(ax, ay + 35, cw, 1, fill=DIVIDER)
            ctx.text(ax + CARD_X, ay + 18, col.name,
                     size=TEXT_BODY, color=ACCENT if focused else TEXT_MID,
                     bold=True, align="left_center")

            # count badge
            bw, bh = 26, 16
            bx = ax + cw - CARD_X - bw
            ctx.rect(bx, ay + 10, bw, bh, fill=BADGE_FILLS[i % len(BADGE_FILLS)], radius=4.0)
            ctx.text(bx + bw / 2, ay + 18, str(len(col.items)),
                     size=TEXT_CAPTION, color=BADGE_TEXT, bold=True, align="center")

            # cards
            scroll_y = ay + CARD_Y
            visible_h = ch - CARD_Y - 4
            stride = CARD_H + CARD_GAP
            scroll_offset = 0.0
            if focused and col.items:
                card_bottom = self._focused_card * stride + CARD_H
                if card_bottom > visible_h:
                    scroll_offset = card_bottom - visible_h

            for idx, item in enumerate(col.items):
                kx = ax + CARD_X
                ky = scroll_y + idx * stride - scroll_offset
                if ky + CARD_H < ay + CARD_Y or ky > ay + ch:
                    continue
                kw = cw - CARD_X * 2
                is_sel = focused and idx == self._focused_card

                bg = CARD_SEL if is_sel else (CARD_FOCUS if focused else CARD_BG)
                ctx.rect(kx, ky, kw, CARD_H, fill=bg, radius=6.0)

                if is_sel:
                    ctx.rect(kx, ky + 4, 3, CARD_H - 8, fill=SEL_BAR, radius=2.0)

                text_x = kx + (14 if is_sel else 10)
                title_max = kw - (14 if is_sel else 10) - 6

                num_str = f"#{item.number}" if item.number else item.itype[:2].upper()
                ctx.text(text_x, ky + 12, num_str,
                         size=TEXT_CAPTION, color=ACCENT if is_sel else TEXT_DIM)
                num_w = len(num_str) * 6 + 4

                ctx.text(text_x + num_w + 4, ky + 12, item.title,
                         size=TEXT_CAPTION, color=TEXT_MID,
                         max_width=title_max - num_w - 4)

                ctx.text(text_x, ky + 30, item.title,
                         size=TEXT_BODY, color=TEXT if is_sel else TEXT_MID,
                         max_width=title_max)

                lx = text_x
                for li, lbl in enumerate(item.labels[:3]):
                    lw = min(len(lbl) * 6 + 10, 80)
                    if lx + lw > ax + cw - CARD_X:
                        break
                    ctx.rect(lx, ky + 50, lw, 14,
                             fill=LABEL_FILLS[li % len(LABEL_FILLS)], radius=3.0)
                    ctx.text(lx + lw / 2, ky + 57, lbl,
                             size=TEXT_CAPTION, color=LABEL_TEXT, align="center",
                             max_width=lw - 4)
                    lx += lw + 4


# ── App ───────────────────────────────────────────────────────────────────────

State = Literal["loading", "error", "picking", "board"]

_BOARD_SHORTCUTS = [
    ("↑↓", "navigate"),
    ("jk", "navigate"),
    ("←→", "column"),
    ("hl", "column"),
    ("Return", "open"),
    ("R", "refresh"),
    ("q", "quit"),
]

_PICK_SHORTCUTS = [
    ("↑↓", "select"),
    ("Return", "open"),
    ("q", "quit"),
]


class GhProjects(App):

    async def on_init(self, ctx: RenderContext) -> None:
        self._state: State = "loading"
        self._error: str = ""
        self._projects: list[Project] = []
        self._project: Project | None = None
        self._columns: list[KanbanColumn] = []
        self._col: int = 0
        self._card: int = 0
        self._loading_msg: str = "Detecting repository…"

        # SelectList for project picker — must be created in on_init
        self._pick_list: SelectList = SelectList([], selected_idx=0)

        self.emit.info(f"gh-projects: init workspace_root={ctx.workspace_root!r}")
        ctx.status_summary("Loading…")

        asyncio.get_event_loop().create_task(self._load(ctx.workspace_root))

    async def _load(self, workspace_root: str) -> None:
        try:
            owner, _name = await asyncio.to_thread(_detect_repo, workspace_root)
            if not owner:
                self._error = "Not a GitHub repo (or gh CLI not available)"
                self._state = "error"
                self.emit.warn(f"gh-projects: repo detection failed at {workspace_root!r}")
                return

            self.emit.info(f"gh-projects: repo={owner}/{_name}, fetching projects…")
            self._loading_msg = f"Fetching projects for {owner}…"

            projects = await asyncio.to_thread(_fetch_projects, owner)
            if not projects:
                self._error = f"No GitHub Projects found for {owner}"
                self._state = "error"
                self.emit.warn(f"gh-projects: no projects found for owner={owner!r}")
                return

            self.emit.info(f"gh-projects: found {len(projects)} project(s)")

            if len(projects) == 1:
                await self._load_board(projects[0])
            else:
                self._projects = projects
                self._pick_list = SelectList(
                    [{"name": p.title, "description": f"#{p.number} · {p.owner}"} for p in projects],
                    selected_idx=0,
                )
                self._state = "picking"
        except Exception as exc:
            self._error = f"Unexpected error: {exc}"
            self._state = "error"
            self.emit.warn(f"gh-projects: load error: {exc}")

    async def _load_board(self, project: Project) -> None:
        self._project = project
        self._state = "loading"
        self._loading_msg = f'Loading "{project.title}"…'
        self.emit.info(f"gh-projects: loading board project={project.number} owner={project.owner!r}")

        columns, err = await asyncio.to_thread(_fetch_board, project)
        if err:
            self._error = err
            self._state = "error"
            self.emit.warn(f"gh-projects: board load failed: {err}")
            return

        total = sum(len(c.items) for c in columns)
        self.emit.info(f"gh-projects: board loaded columns={len(columns)} items={total}")
        self._columns = columns
        self._col = 0
        self._card = 0
        self._clamp()
        self._state = "board"

    # ── helpers ───────────────────────────────────────────────────────────────

    def _clamp(self) -> None:
        if not self._columns:
            return
        self._col = max(0, min(self._col, len(self._columns) - 1))
        cards = self._columns[self._col].items
        self._card = max(0, min(self._card, len(cards) - 1)) if cards else 0

    def _focused_item(self) -> Item | None:
        if not self._columns:
            return None
        col = self._columns[self._col]
        if not col.items:
            return None
        return col.items[self._card]

    def _update_status(self, ctx: RenderContext) -> None:
        item = self._focused_item()
        if item:
            label = f"#{item.number}" if item.number else item.itype
            ctx.status_summary(f"{self._columns[self._col].name} · {label}")
        elif self._columns:
            ctx.status_summary(self._columns[self._col].name)

    # ── render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        if self._state == "loading":
            self._render_loading(ctx)
        elif self._state == "error":
            self._render_error(ctx)
        elif self._state == "picking":
            self._render_picker(ctx)
        else:
            self._render_board(ctx)

    def _render_loading(self, ctx: RenderContext) -> None:
        ctx.render(Column([
            AppBar("GitHub Projects"),
            Spacer(grow=True),
            Label(self._loading_msg, tone="hint", color=TEXT_DIM),
            Spacer(grow=True),
        ]))

    def _render_error(self, ctx: RenderContext) -> None:
        ctx.render(Column([
            AppBar("GitHub Projects"),
            Spacer(grow=True),
            Label("Error", tone="body", color=ERROR_COL, bold=True),
            Label(self._error, tone="caption", color=TEXT_MID),
            Label("Press R to retry", tone="hint", color=TEXT_DIM),
            Spacer(grow=True),
        ]))

    def _render_picker(self, ctx: RenderContext) -> None:
        ctx.render(Column([
            AppBar("GitHub Projects", subtitle="Select a project"),
            self._pick_list,
            FooterKeys(_PICK_SHORTCUTS),
        ], padding=SPACE_SM, padding_top=0))

    def _render_board(self, ctx: RenderContext) -> None:
        project_title = self._project.title if self._project else "GitHub Projects"
        total = sum(len(c.items) for c in self._columns)
        ctx.render(Column([
            AppBar(project_title, subtitle=f"{total} items"),
            KanbanBody(self._columns, self._col, self._card),
            FooterKeys(_BOARD_SHORTCUTS),
        ], padding=0, padding_top=0))

    # ── keyboard ──────────────────────────────────────────────────────────────

    async def on_key(self, ctx: RenderContext, key: str, _mods: dict) -> None:
        if key == "q":
            sys.exit(0)

        if key == "r" or key == "R":
            self.emit.info("gh-projects: user triggered refresh")
            if self._project:
                asyncio.get_event_loop().create_task(self._load_board(self._project))
            else:
                self._state = "loading"
                self._loading_msg = "Detecting repository…"
                asyncio.get_event_loop().create_task(self._load(ctx.workspace_root))
            return

        if self._state == "picking":
            if key in ("up", "k", "down", "j"):
                self._pick_list.handle_key(key)
            elif key == "return":
                idx = self._pick_list.selected_idx
                self.emit.info(f"gh-projects: user selected project #{self._projects[idx].number}")
                asyncio.get_event_loop().create_task(
                    self._load_board(self._projects[idx])
                )
            return

        if self._state != "board":
            return

        if key in ("up", "k"):
            self._card = max(0, self._card - 1)
        elif key in ("down", "j"):
            col = self._columns[self._col]
            self._card = min(len(col.items) - 1, self._card + 1) if col.items else 0
        elif key in ("left", "h"):
            self._col = max(0, self._col - 1)
            self._clamp()
        elif key in ("right", "l"):
            self._col = min(len(self._columns) - 1, self._col + 1)
            self._clamp()
        elif key == "return":
            item = self._focused_item()
            if item and item.url:
                self.emit.info(f"gh-projects: opening {item.url}")
                await asyncio.to_thread(_open_url, item.url)
            return

        self._update_status(ctx)


GhProjects().run()
