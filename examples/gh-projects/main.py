#!/usr/bin/env python3
"""gh-projects — read-only GitHub Projects Kanban board."""
from __future__ import annotations
import asyncio
import json
import subprocess
import sys
from dataclasses import dataclass, field
from typing import Literal, cast

from plexi_sdk import App, RenderContext, CAPTION, BODY, TITLE

# ── Palette ───────────────────────────────────────────────────────────────────
APP_BG     = "#0d0d12"
COL_BG     = "#14141c"
COL_ACTIVE = "#18182a"
DIVIDER    = "#252535"
CARD_BG    = "#1c1c28"
CARD_FOCUS = "#21213a"
CARD_SEL   = "#282850"
TEXT       = "#dcdcf0"
TEXT_MID   = "#8888a8"
TEXT_DIM   = "#505068"
ACCENT     = "#7b9ef0"
TAG_BG     = "#22224a"
SEL_BAR    = "#5068d8"
ERROR_COL  = "#d04050"
OK_COL     = "#50c080"

LABEL_FILLS = ["#2e3060", "#1e3a70", "#1a4a36", "#3a2060", "#603020"]
LABEL_TEXT  = "#aab0e0"

BADGE_FILLS = ["#2e3060", "#1e3a70", "#1a4a36"]
BADGE_TEXT  = "#aab0e0"

# ── Layout constants ──────────────────────────────────────────────────────────
HEADER_H = 44
STATUS_H = 30
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
class Column:
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


def _fetch_board(project: Project) -> tuple[list[Column], str]:
    """
    Return (columns, error_msg). columns is empty on error.
    Fetches field options to determine column order, then items.
    """
    fields = _run_gh(
        "project", "field-list", str(project.number),
        "--owner", project.owner, "--format", "json",
    )
    if fields is None:
        return [], "Failed to fetch project fields"

    status_options: list[dict[str, object]] = []
    for f in cast(list[dict[str, object]], fields.get("fields") or []):
        if f.get("name") == "Status" and "SingleSelect" in str(f.get("type") or ""):
            status_options = cast(list[dict[str, object]], f.get("options") or [])
            break

    if not status_options:
        return [], "No Status field found on project"

    items_data = _run_gh(
        "project", "item-list", str(project.number),
        "--owner", project.owner, "--format", "json", "--limit", "500",
    )
    if items_data is None:
        return [], "Failed to fetch project items"

    col_map: dict[str, list[Item]] = {str(opt.get("name", "")): [] for opt in status_options if opt.get("name")}

    for raw in cast(list[dict[str, object]], items_data.get("items") or []):
        content = cast(dict[str, object], raw.get("content") or {})
        itype = str(raw.get("type") or "DraftIssue")
        status = str(raw.get("status") or "").strip()

        number_raw = content.get("number")
        try:
            number = int(number_raw) if number_raw is not None else None  # type: ignore[arg-type]
        except (ValueError, TypeError):
            number = None
        title  = str(raw.get("title") or content.get("title") or "(no title)")
        url    = str(content.get("url") or "")
        labels: list[str] = []
        for lbl in cast(list[object], content.get("labels") or []):
            name = lbl.get("name") if isinstance(lbl, dict) else lbl  # type: ignore[union-attr]
            if isinstance(name, str) and name:
                labels.append(name)

        item = Item(
            item_id=str(raw.get("id") or ""),
            title=title,
            number=number,
            itype=itype,
            labels=labels,
            url=url,
        )
        if status in col_map:
            col_map[status].append(item)

    columns = [
        Column(name=str(opt.get("name", "")), items=col_map.get(str(opt.get("name", "")), []))
        for opt in status_options if opt.get("name")
    ]
    return columns, ""


def _fetch_projects(owner: str) -> list[Project]:
    data = _run_gh("project", "list", "--owner", owner, "--format", "json", "--limit", "50")
    if data is None:
        return []
    projects = []
    for p in cast(list[dict[str, object]], data.get("projects") or []):
        projects.append(Project(
            number=int(p.get("number") or 0),  # type: ignore[arg-type]
            title=str(p.get("title") or ""),
            owner=owner,
        ))
    return projects


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


# ── App ───────────────────────────────────────────────────────────────────────

State = Literal["loading", "error", "picking", "board"]


class GhProjects(App):

    async def on_init(self, ctx: RenderContext) -> None:
        self._state: State = "loading"
        self._error: str = ""
        self._projects: list[Project] = []
        self._pick_idx: int = 0
        self._project: Project | None = None
        self._columns: list[Column] = []
        self._col: int = 0
        self._card: int = 0
        self._loading_msg: str = "Detecting repository…"

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

    def _col_rect(self, ctx: RenderContext, i: int) -> tuple[float, float, float, float]:
        n = max(1, len(self._columns))
        w = (ctx.w - COL_PAD * 2 - COL_GAP * (n - 1)) / n
        x = COL_PAD + i * (w + COL_GAP)
        return x, float(HEADER_H), w, ctx.h - HEADER_H - STATUS_H

    def _update_status(self, ctx: RenderContext) -> None:
        item = self._focused_item()
        if item:
            label = f"#{item.number}" if item.number else item.itype
            ctx.status_summary(f"{self._columns[self._col].name} · {label}")
        elif self._columns:
            ctx.status_summary(self._columns[self._col].name)

    # ── render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(APP_BG)

        if self._state == "loading":
            self._render_loading(ctx)
        elif self._state == "error":
            self._render_error(ctx)
        elif self._state == "picking":
            self._render_picker(ctx)
        else:
            self._render_board(ctx)

    def _render_loading(self, ctx: RenderContext) -> None:
        ctx.text(ctx.w / 2, ctx.h / 2, self._loading_msg,
                 size=BODY, color=TEXT_DIM, align="center")

    def _render_error(self, ctx: RenderContext) -> None:
        ctx.text(ctx.w / 2, ctx.h / 2 - 12, "Error", size=BODY, color=ERROR_COL,
                 bold=True, align="center")
        ctx.text(ctx.w / 2, ctx.h / 2 + 10, self._error,
                 size=CAPTION, color=TEXT_MID, align="center", max_width=ctx.w - 40)
        ctx.text(ctx.w / 2, ctx.h / 2 + 32, "Press R to retry",
                 size=CAPTION, color=TEXT_DIM, align="center")

    def _render_picker(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, HEADER_H, fill=COL_BG)
        ctx.rect(0, HEADER_H - 1, ctx.w, 1, fill=DIVIDER)
        ctx.text(COL_PAD + 2, HEADER_H / 2, "GitHub Projects",
                 size=TITLE, color=TEXT, bold=True, align="left_center")

        for i, proj in enumerate(self._projects):
            y = HEADER_H + 10 + i * 44
            is_sel = (i == self._pick_idx)
            bg = CARD_SEL if is_sel else CARD_BG
            ctx.rect(COL_PAD, y, ctx.w - COL_PAD * 2, 38, fill=bg, radius=6.0)
            if is_sel:
                ctx.rect(COL_PAD, y + 4, 3, 30, fill=SEL_BAR, radius=2.0)
            tx = COL_PAD + (18 if is_sel else 12)
            ctx.text(tx, y + 10, proj.title,
                     size=BODY, color=TEXT if is_sel else TEXT_MID)
            ctx.text(tx, y + 26, f"#{proj.number} · {proj.owner}",
                     size=CAPTION, color=ACCENT if is_sel else TEXT_DIM)

        ctx.rect(0, ctx.h - STATUS_H, ctx.w, STATUS_H, fill=COL_BG)
        ctx.rect(0, ctx.h - STATUS_H, ctx.w, 1, fill=DIVIDER)
        ctx.text(ctx.w / 2, ctx.h - STATUS_H / 2,
                 "↑↓  select   Return  open   q  quit",
                 size=CAPTION, color=TEXT_DIM, align="center")

    def _render_board(self, ctx: RenderContext) -> None:
        # header
        project_title = self._project.title if self._project else "GitHub Projects"
        ctx.rect(0, 0, ctx.w, HEADER_H, fill=COL_BG)
        ctx.rect(0, HEADER_H - 1, ctx.w, 1, fill=DIVIDER)
        ctx.text(COL_PAD + 2, HEADER_H / 2, project_title,
                 size=TITLE, color=TEXT, bold=True, align="left_center")
        total = sum(len(c.items) for c in self._columns)
        ctx.text(ctx.w - COL_PAD, HEADER_H / 2, f"{total} items",
                 size=CAPTION, color=TEXT_DIM, align="right_center")

        for i, col in enumerate(self._columns):
            cx, cy, cw, ch = self._col_rect(ctx, i)
            focused = (i == self._col)

            ctx.rect(cx, cy, cw, ch, fill=COL_ACTIVE if focused else COL_BG, radius=COL_R)

            # column header
            hdr_bg = "#1e1e38" if focused else "#161622"
            ctx.rect(cx, cy, cw, 36, fill=hdr_bg, radius=COL_R)
            ctx.rect(cx, cy + 28, cw, 8, fill=hdr_bg)
            ctx.rect(cx, cy + 35, cw, 1, fill=DIVIDER)
            ctx.text(cx + CARD_X, cy + 18, col.name,
                     size=BODY, color=ACCENT if focused else TEXT_MID,
                     bold=True, align="left_center")

            # count badge
            bw, bh = 26, 16
            bx = cx + cw - CARD_X - bw
            ctx.rect(bx, cy + 10, bw, bh, fill=BADGE_FILLS[i % len(BADGE_FILLS)], radius=4.0)
            ctx.text(bx + bw / 2, cy + 18, str(len(col.items)),
                     size=CAPTION, color=BADGE_TEXT, bold=True, align="center")

            # cards
            scroll_y = cy + CARD_Y
            visible_h = ch - CARD_Y - 4
            stride = CARD_H + CARD_GAP
            scroll_offset = 0.0
            if focused and col.items:
                card_bottom = self._card * stride + CARD_H
                if card_bottom > visible_h:
                    scroll_offset = card_bottom - visible_h

            for idx, item in enumerate(col.items):
                kx = cx + CARD_X
                ky = scroll_y + idx * stride - scroll_offset
                if ky + CARD_H < cy + CARD_Y or ky > cy + ch:
                    continue
                kw = cw - CARD_X * 2
                is_sel = focused and idx == self._card

                bg = CARD_SEL if is_sel else (CARD_FOCUS if focused else CARD_BG)
                ctx.rect(kx, ky, kw, CARD_H, fill=bg, radius=6.0)

                if is_sel:
                    ctx.rect(kx, ky + 4, 3, CARD_H - 8, fill=SEL_BAR, radius=2.0)

                text_x = kx + (14 if is_sel else 10)
                title_max = kw - (14 if is_sel else 10) - 6

                # number chip
                num_str = f"#{item.number}" if item.number else item.itype[:2].upper()
                ctx.text(text_x, ky + 12, num_str,
                         size=CAPTION, color=ACCENT if is_sel else TEXT_DIM)
                num_w = len(num_str) * 6 + 4

                ctx.text(text_x + num_w + 4, ky + 12, item.title,
                         size=CAPTION, color=TEXT_MID,
                         max_width=title_max - num_w - 4)

                ctx.text(text_x, ky + 30, item.title,
                         size=BODY, color=TEXT if is_sel else TEXT_MID,
                         max_width=title_max)

                # labels
                lx = text_x
                for li, lbl in enumerate(item.labels[:3]):
                    lw = min(len(lbl) * 6 + 10, 80)
                    if lx + lw > cx + cw - CARD_X:
                        break
                    ctx.rect(lx, ky + 50, lw, 14,
                              fill=LABEL_FILLS[li % len(LABEL_FILLS)], radius=3.0)
                    ctx.text(lx + lw / 2, ky + 57, lbl,
                              size=CAPTION, color=LABEL_TEXT, align="center",
                              max_width=lw - 4)
                    lx += lw + 4

        # status bar
        ctx.rect(0, ctx.h - STATUS_H, ctx.w, STATUS_H, fill=COL_BG)
        ctx.rect(0, ctx.h - STATUS_H, ctx.w, 1, fill=DIVIDER)
        ctx.text(ctx.w / 2, ctx.h - STATUS_H / 2,
                 "↑↓/jk  navigate   ←→/hl  column   Return  open in browser   R  refresh   q  quit",
                 size=CAPTION, color=TEXT_DIM, align="center")

    # ── keyboard ──────────────────────────────────────────────────────────────

    async def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
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
            if key in ("up", "k"):
                self._pick_idx = max(0, self._pick_idx - 1)
            elif key in ("down", "j"):
                self._pick_idx = min(len(self._projects) - 1, self._pick_idx + 1)
            elif key == "return":
                self.emit.info(f"gh-projects: user selected project #{self._projects[self._pick_idx].number}")
                asyncio.get_event_loop().create_task(
                    self._load_board(self._projects[self._pick_idx])
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
