#!/usr/bin/env python3
"""gh-triage — Keyboard-driven GitHub issue triage.

One issue at a time. Toggle labels, skip, or advance. Configurable criteria
determine what counts as needing triage.

Keys:
  n/→     next issue        b/←     back
  j/k     navigate labels   tab     cycle section
  enter   toggle label      space   toggle label
  p       priority picker   l       load picker    a  area picker
  o       open in browser   r       refresh        esc  close picker
"""
from __future__ import annotations

import asyncio
import json
import subprocess
from dataclasses import dataclass
from typing import Literal

from plexi_sdk import (
    App, RenderContext,
    BG, SURFACE, FG, ACCENT, MUTED, HIGHLIGHT,
    BODY, CAPTION, HEADING, HINT,
    GREEN, RED, YELLOW,
    PAD, PAD_TIGHT,
)

# ── Layout ────────────────────────────────────────────────────────────────────

HEADER_H    = 52.0
ISSUE_HDR_H = 64.0   # number + title row
LABEL_ROW_H = 36.0   # one label section row (section title)
CHIP_H      = 24.0
CHIP_PAD_X  = 10.0
CHIP_GAP    = 6.0
SECTION_SEP = 10.0
FOOTER_H    = 32.0
PICKER_ROW  = 36.0

# ── Colors ────────────────────────────────────────────────────────────────────

ORANGE = "#fab387"
BLUE   = "#89b4fa"

# ── Triage criteria ───────────────────────────────────────────────────────────

TRIAGE_FLAGS   = {"needs-triage", "untriaged"}
PRIORITY_PFXS  = {"p0", "p1", "p2", "p3", "p4"}
LOAD_PFX       = "load:"
AREA_PFX       = "area:"

PRIORITY_LABELS = ["P0", "P1", "P2", "P3", "P4"]
LOAD_LABELS     = ["load:XS", "load:S", "load:M", "load:L", "load:XL"]


def _needs_triage(labels: list[str]) -> bool:
    names = {l.lower() for l in labels}
    if names & TRIAGE_FLAGS:
        return True
    if not any(n in PRIORITY_PFXS for n in names):
        return True
    if not any(n.startswith(LOAD_PFX) for n in names):
        return True
    if not any(n.startswith(AREA_PFX) for n in names):
        return True
    return False


def _missing_criteria(labels: list[str]) -> list[str]:
    names = {l.lower() for l in labels}
    missing = []
    if names & TRIAGE_FLAGS:
        missing.append("has triage flag")
    if not any(n in PRIORITY_PFXS for n in names):
        missing.append("no priority")
    if not any(n.startswith(LOAD_PFX) for n in names):
        missing.append("no load")
    if not any(n.startswith(AREA_PFX) for n in names):
        missing.append("no area")
    return missing


# ── gh helpers ────────────────────────────────────────────────────────────────

def _gh(*args: str, cwd: str | None = None, timeout: float = 20.0) -> tuple[int, str, str]:
    try:
        p = subprocess.run(
            ["gh", *args], cwd=cwd, capture_output=True, text=True, timeout=timeout,
        )
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return -1, "", "timeout"
    except Exception as exc:
        return -1, "", str(exc)


def _load_issues(root: str | None) -> tuple[list[dict], str]:
    rc, out, err = _gh(
        "issue", "list", "--state", "open",
        "--json", "number,title,labels,body",
        "--limit", "200",
        cwd=root,
    )
    if rc != 0:
        return [], err.strip() or f"gh exit {rc}"
    try:
        issues = json.loads(out)
        return [i for i in issues if _needs_triage([l["name"] for l in i.get("labels", [])])], ""
    except Exception as exc:
        return [], str(exc)


def _load_labels(root: str | None) -> tuple[list[str], str]:
    rc, out, err = _gh(
        "label", "list",
        "--json", "name",
        "--limit", "200",
        cwd=root,
    )
    if rc != 0:
        return [], err.strip() or f"gh exit {rc}"
    try:
        return [l["name"] for l in json.loads(out)], ""
    except Exception as exc:
        return [], str(exc)


def _add_label(number: int, label: str, root: str | None) -> str:
    rc, _, err = _gh("issue", "edit", str(number), "--add-label", label, cwd=root)
    return "" if rc == 0 else err.strip() or f"exit {rc}"


def _remove_label(number: int, label: str, root: str | None) -> str:
    rc, _, err = _gh("issue", "edit", str(number), "--remove-label", label, cwd=root)
    return "" if rc == 0 else err.strip() or f"exit {rc}"


# ── Data model ────────────────────────────────────────────────────────────────

@dataclass
class Issue:
    number: int
    title: str
    body: str
    labels: list[str]


def _label_color(name: str) -> str:
    n = name.lower()
    if n in ("p0",):
        return RED
    if n in ("p1",):
        return ORANGE
    if n in ("p2",):
        return YELLOW
    if n.startswith("area:"):
        return BLUE
    if n.startswith("load:"):
        return "#b4befe"  # lavender
    if n in TRIAGE_FLAGS:
        return RED
    if n in ("p3", "p4", "ready"):
        return GREEN
    return ACCENT


# ── App ───────────────────────────────────────────────────────────────────────

Section = Literal["current", "available"]
State   = Literal["loading", "triage", "done", "error"]


class GhTriage(App):

    async def on_init(self, ctx: RenderContext) -> None:
        self._state:    State   = "loading"
        self._error:    str     = ""
        self._issues:   list[Issue] = []
        self._all_labels: list[str] = []
        self._idx:      int     = 0
        self._history:  list[int] = []   # indices visited (for back)
        self._triaged:  int     = 0      # count triaged this session
        self._total:    int     = 0      # total needing triage at load time
        self._root:     str     = ctx.workspace_root or ""

        # UI state
        self._section:  Section = "current"
        self._label_sel: int    = 0
        self._scroll_y: float   = 0.0
        self._picker:   list[str] | None = None   # None = no picker
        self._picker_sel: int   = 0
        self._pending:  str     = ""   # pending async op description
        self._flash:    str     = ""   # transient feedback message

        self.emit.info(f"gh-triage: init workspace={self._root!r}")
        ctx.status_summary("Loading…")
        asyncio.get_event_loop().create_task(self._load())

    # ── data ──────────────────────────────────────────────────────────────────

    async def _load(self) -> None:
        self._state = "loading"
        self.emit.schedule_render()

        issues_raw, labels_raw = await asyncio.gather(
            asyncio.to_thread(_load_issues, self._root or None),
            asyncio.to_thread(_load_labels, self._root or None),
        )
        issues, err1 = issues_raw
        all_labels, err2 = labels_raw

        if err1:
            self.emit.warn(f"gh-triage: load issues failed: {err1}")
            self._error = err1
            self._state = "error"
            self.emit.schedule_render()
            return

        if err2:
            self.emit.warn(f"gh-triage: load labels failed (non-fatal): {err2}")

        self._issues = [
            Issue(
                number=i["number"],
                title=i["title"],
                body=(i.get("body") or "").strip(),
                labels=[l["name"] for l in i.get("labels", [])],
            )
            for i in issues
        ]
        self._all_labels = all_labels
        self._total = len(self._issues)
        self._idx = 0
        self._history = []
        self._label_sel = 0
        self._section = "current"

        self.emit.info(f"gh-triage: loaded {self._total} issues needing triage, {len(all_labels)} labels")

        if self._issues:
            self._state = "triage"
            self.emit.schedule_render()
        else:
            self._state = "done"
            self.emit.schedule_render()

    def _current(self) -> Issue | None:
        if not self._issues or self._idx >= len(self._issues):
            return None
        return self._issues[self._idx]

    def _available_labels(self, issue: Issue) -> list[str]:
        """Labels not on this issue, sorted: missing-criteria groups first."""
        current = set(issue.labels)
        missing_names = {l.lower() for l in issue.labels}

        needs_priority = not any(n in PRIORITY_PFXS for n in missing_names)
        needs_load     = not any(n.startswith(LOAD_PFX) for n in missing_names)
        needs_area     = not any(n.startswith(AREA_PFX) for n in missing_names)

        priority_candidates = [l for l in PRIORITY_LABELS if l not in current]
        load_candidates     = [l for l in LOAD_LABELS if l not in current]
        area_candidates     = [l for l in self._all_labels
                               if l.lower().startswith(AREA_PFX) and l not in current]
        other               = [l for l in self._all_labels
                               if l not in current
                               and l not in priority_candidates
                               and l not in load_candidates
                               and l not in area_candidates]

        result: list[str] = []
        if needs_priority:
            result.extend(priority_candidates)
        if needs_load:
            result.extend(load_candidates)
        if needs_area:
            result.extend(area_candidates[:8])
        for l in priority_candidates + load_candidates:
            if not needs_priority and l in priority_candidates:
                result.append(l)
            if not needs_load and l in load_candidates:
                result.append(l)
        result.extend(other[:20])
        # deduplicate while preserving order
        seen: set[str] = set()
        deduped: list[str] = []
        for l in result:
            if l not in seen:
                seen.add(l)
                deduped.append(l)
        return deduped

    # ── async label mutations ─────────────────────────────────────────────────

    def _toggle_label(self, label: str) -> None:
        issue = self._current()
        if issue is None:
            return
        if label in issue.labels:
            self._pending = f"Removing {label}…"
            asyncio.get_event_loop().create_task(self._do_remove(issue.number, label))
        else:
            self._pending = f"Adding {label}…"
            asyncio.get_event_loop().create_task(self._do_add(issue.number, label))
        self.emit.schedule_render()

    async def _do_add(self, number: int, label: str) -> None:
        self.emit.info(f"gh-triage: add label #{number} label={label!r}")
        err = await asyncio.to_thread(_add_label, number, label, self._root or None)
        if err:
            self.emit.warn(f"gh-triage: add label failed #{number} label={label!r}: {err}")
            self._flash = f"Error: {err}"
        else:
            # update local state
            issue = next((i for i in self._issues if i.number == number), None)
            if issue and label not in issue.labels:
                issue.labels.append(label)
            self._flash = f"Added {label}"
            self.emit.info(f"gh-triage: added label #{number} label={label!r}")
        self._pending = ""
        self.emit.schedule_render()

    async def _do_remove(self, number: int, label: str) -> None:
        self.emit.info(f"gh-triage: remove label #{number} label={label!r}")
        err = await asyncio.to_thread(_remove_label, number, label, self._root or None)
        if err:
            self.emit.warn(f"gh-triage: remove label failed #{number} label={label!r}: {err}")
            self._flash = f"Error: {err}"
        else:
            issue = next((i for i in self._issues if i.number == number), None)
            if issue and label in issue.labels:
                issue.labels.remove(label)
            self._flash = f"Removed {label}"
            self.emit.info(f"gh-triage: removed label #{number} label={label!r}")
        self._pending = ""
        self.emit.schedule_render()

    # ── navigation ────────────────────────────────────────────────────────────

    def _advance(self) -> None:
        """Move to next issue; mark current as triaged if criteria met."""
        issue = self._current()
        if issue and not _needs_triage(issue.labels):
            self._triaged += 1
            self.emit.info(f"gh-triage: #{issue.number} triaged; session count={self._triaged}")

        self._history.append(self._idx)
        self._idx += 1
        self._label_sel = 0
        self._section = "current"
        self._picker = None
        self._flash = ""
        self._scroll_y = 0.0

        if self._idx >= len(self._issues):
            self._state = "done"
            self.emit.info(f"gh-triage: queue exhausted triaged={self._triaged}/{self._total}")
        self.emit.schedule_render()

    def _go_back(self) -> None:
        if not self._history:
            return
        self._idx = self._history.pop()
        self._label_sel = 0
        self._section = "current"
        self._picker = None
        self._flash = ""
        self._scroll_y = 0.0
        if self._state == "done":
            self._state = "triage"
        self.emit.info(f"gh-triage: back to idx={self._idx}")
        self.emit.schedule_render()

    # ── render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)
        self._draw_header(ctx)

        if self._state == "loading":
            ctx.text(PAD, HEADER_H + PAD, "Loading issues…", size=BODY, color=MUTED)
            return
        if self._state == "error":
            ctx.text(PAD, HEADER_H + PAD, f"Error: {self._error}",
                     size=CAPTION, color=RED, max_width=ctx.w - PAD * 2)
            ctx.text(PAD, HEADER_H + PAD * 2 + CAPTION, "r — retry", size=HINT, color=MUTED)
            return
        if self._state == "done":
            self._draw_done(ctx)
            return

        self._draw_issue(ctx)

        if self._picker is not None:
            self._draw_picker(ctx)

    def _draw_header(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, HEADER_H, fill=SURFACE)
        ctx.rect(0, HEADER_H - 1, ctx.w, 1, fill=BG)
        mid_y = HEADER_H / 2

        ctx.text(PAD, mid_y - HEADING / 2, "Triage",
                 size=HEADING, color=ACCENT, bold=True, monospace=True)

        if self._state == "triage" and self._issues:
            remaining = len(self._issues) - self._idx
            label = f"{self._triaged} ✓  {remaining} left"
            ctx.badge(x=PAD + 82, y_center=mid_y,
                      label=label, fill=SURFACE, fg=MUTED, font_size=HINT, radius=3.0)

        ctx.shortcuts(
            x=ctx.w * 0.42, y=mid_y - HINT / 2,
            max_width=ctx.w * 0.58 - PAD,
            pairs=[
                ("n", "next"), ("b", "back"), ("p", "priority"),
                ("l", "load"), ("a", "area"), ("o", "browser"),
            ],
        )

    def _draw_done(self, ctx: RenderContext) -> None:
        cy = ctx.h / 2
        ctx.text(ctx.w / 2, cy - HEADING, "All done!",
                 size=HEADING, color=GREEN, bold=True, align="center")
        ctx.text(ctx.w / 2, cy, f"Triaged {self._triaged} of {self._total} issues this session.",
                 size=BODY, color=FG, align="center")
        ctx.text(ctx.w / 2, cy + BODY + PAD, "r — refresh to check for new issues",
                 size=HINT, color=MUTED, align="center")

    def _draw_issue(self, ctx: RenderContext) -> None:
        issue = self._current()
        if issue is None:
            return

        y = HEADER_H + PAD

        # issue number + title
        ctx.badge(x=PAD, y_center=y + ISSUE_HDR_H / 2 - 4,
                  label=f"#{issue.number}",
                  fill=ACCENT, fg=BG, font_size=CAPTION, radius=4.0)
        ctx.text(PAD + 56, y + 6, issue.title,
                 size=BODY, color=FG, bold=True, max_width=ctx.w - PAD - 56 - PAD)
        missing = _missing_criteria(issue.labels)
        if missing:
            ctx.text(PAD + 56, y + 6 + BODY + 4,
                     "Missing: " + ", ".join(missing),
                     size=HINT, color=YELLOW, max_width=ctx.w - PAD - 56 - PAD)
        y += ISSUE_HDR_H

        # divider
        ctx.rect(PAD, y, ctx.w - PAD * 2, 1, fill=SURFACE)
        y += PAD_TIGHT

        # compute label panel height so body gets remaining space
        cur_issue = issue
        current_labels = cur_issue.labels
        available = self._available_labels(cur_issue)
        label_panel_h = self._label_panel_height()

        body_h = max(ctx.h - y - label_panel_h - FOOTER_H - PAD, 60.0)
        body_content_h = max(len(issue.body.splitlines()) * (CAPTION + 2) + PAD, body_h)

        if issue.body:
            ctx.begin_scroll("body", 0.0, y, ctx.w, body_h, body_content_h)
            ctx.markdown(PAD, y - self._scroll_y, ctx.w - PAD * 2, issue.body)
            ctx.end_scroll()
        else:
            ctx.text(PAD, y + PAD_TIGHT, "No description.", size=CAPTION, color=MUTED)
        y += body_h + PAD_TIGHT

        ctx.rect(PAD, y, ctx.w - PAD * 2, 1, fill=SURFACE)
        y += PAD_TIGHT

        # label panel
        self._draw_label_panel(ctx, y, current_labels, available)

        # footer
        fy = ctx.h - FOOTER_H
        ctx.rect(0, fy, ctx.w, 1, fill=SURFACE)
        status = self._pending or self._flash or ""
        if status:
            col = YELLOW if self._pending else (RED if "Error" in status else GREEN)
            ctx.text(PAD, fy + (FOOTER_H - HINT) / 2, status, size=HINT, color=col)
        ctx.shortcuts(
            x=ctx.w - PAD, y=fy + (FOOTER_H - HINT) / 2,
            max_width=ctx.w - PAD * 2 - 200,
            pairs=[
                ("tab", "switch section"), ("enter", "toggle"),
                ("n", "next"), ("b", "back"),
            ],
        )

    def _label_panel_height(self) -> float:
        return LABEL_ROW_H + CHIP_H + PAD_TIGHT + SECTION_SEP + LABEL_ROW_H + CHIP_H + PAD

    def _draw_label_panel(self, ctx: RenderContext, y: float,
                          current: list[str], available: list[str]) -> None:
        # Current labels section
        sec_cur = (self._section == "current")
        ctx.text(PAD, y + (LABEL_ROW_H - CAPTION) / 2,
                 "CURRENT", size=CAPTION, color=ACCENT if sec_cur else MUTED,
                 bold=sec_cur, monospace=True)
        y += LABEL_ROW_H

        self._draw_chips(ctx, y, current, "current")
        y += CHIP_H + PAD_TIGHT + SECTION_SEP

        # Available labels section
        sec_avail = (self._section == "available")
        ctx.text(PAD, y + (LABEL_ROW_H - CAPTION) / 2,
                 "ADD", size=CAPTION, color=ACCENT if sec_avail else MUTED,
                 bold=sec_avail, monospace=True)
        y += LABEL_ROW_H

        self._draw_chips(ctx, y, available[:18], "available")

    def _draw_chips(self, ctx: RenderContext, y: float,
                    labels: list[str], section: str) -> None:
        focused_section = (self._section == section)
        x = PAD
        for i, label in enumerate(labels):
            w = len(label) * 7.5 + CHIP_PAD_X * 2
            if x + w > ctx.w - PAD:
                break  # single row for now

            is_sel = focused_section and (i == self._label_sel)
            fill   = HIGHLIGHT if is_sel else SURFACE
            col    = _label_color(label)

            if is_sel:
                ctx.rect(x - 2, y - 2, w + 4, CHIP_H + 4, fill=ACCENT, radius=5.0)
            ctx.rect(x, y, w, CHIP_H, fill=fill, radius=4.0)
            ctx.rect(x, y, 3, CHIP_H, fill=col, radius=4.0)
            ctx.text(x + CHIP_PAD_X + 2, y + (CHIP_H - HINT) / 2,
                     label, size=HINT, color=FG if is_sel else col)

            # store chip rect for click detection (approximate via _chip_rects)
            x += w + CHIP_GAP

        if not labels:
            ctx.text(PAD, y + (CHIP_H - HINT) / 2,
                     "(none)" if section == "current" else "(loading…)",
                     size=HINT, color=MUTED)

    def _draw_picker(self, ctx: RenderContext) -> None:
        opts = self._picker
        if opts is None:
            return
        pw   = min(280.0, ctx.w - PAD * 4)
        ph   = PAD_TIGHT + PICKER_ROW * len(opts) + PAD_TIGHT
        px   = (ctx.w - pw) / 2
        py   = HEADER_H + PAD + ISSUE_HDR_H

        ctx.rect(px - 2, py - 2, pw + 4, ph + 4, fill=ACCENT, radius=9.0)
        ctx.rect(px, py, pw, ph, fill=SURFACE, radius=8.0)

        for i, opt in enumerate(opts):
            ry = py + PAD_TIGHT + i * PICKER_ROW
            is_sel = (i == self._picker_sel)
            if is_sel:
                ctx.rect(px + 4, ry, pw - 8, PICKER_ROW - 2, fill=HIGHLIGHT, radius=5.0)
            col = _label_color(opt)
            ctx.rect(px + 8, ry + (PICKER_ROW - CHIP_H) / 2, 3, CHIP_H, fill=col, radius=2.0)
            ctx.text(px + 16, ry + (PICKER_ROW - BODY) / 2,
                     opt, size=BODY, color=FG if is_sel else MUTED)

    # ── chips for click detection ─────────────────────────────────────────────

    def _chip_rects(self, y_start: float, labels: list[str]) -> list[tuple[float, float, float, float, str]]:
        """Return list of (x, y, w, h, label) for chip hit testing."""
        rects = []
        x = PAD
        for label in labels:
            w = len(label) * 7.5 + CHIP_PAD_X * 2
            if x + w > 9999:
                break
            rects.append((x, y_start, w, CHIP_H, label))
            x += w + CHIP_GAP
        return rects

    def _label_section_y(self, ctx: RenderContext) -> tuple[float, float]:
        """Return (current_chips_y, available_chips_y)."""
        y = HEADER_H + PAD + ISSUE_HDR_H + PAD_TIGHT + PAD_TIGHT
        label_panel_h = self._label_panel_height()
        body_h = max(ctx.h - y - label_panel_h - FOOTER_H - PAD, 60.0)
        panel_y = y + body_h + PAD_TIGHT + PAD_TIGHT
        cur_chips_y  = panel_y + LABEL_ROW_H
        avail_chips_y = cur_chips_y + CHIP_H + PAD_TIGHT + SECTION_SEP + LABEL_ROW_H
        return cur_chips_y, avail_chips_y

    # ── input ─────────────────────────────────────────────────────────────────

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        if self._state == "loading":
            return

        if self._state == "error":
            if key == "r":
                asyncio.get_event_loop().create_task(self._load())
            return

        if self._state == "done":
            if key == "r":
                asyncio.get_event_loop().create_task(self._load())
            elif key in ("b", "left"):
                self._go_back()
            return

        # picker is open
        if self._picker is not None:
            if key in ("escape", "q"):
                self.emit.info("gh-triage: picker closed")
                self._picker = None
                self.emit.schedule_render()
            elif key in ("up", "k"):
                self._picker_sel = max(0, self._picker_sel - 1)
                self.emit.schedule_render()
            elif key in ("down", "j"):
                self._picker_sel = min(len(self._picker) - 1, self._picker_sel + 1)
                self.emit.schedule_render()
            elif key in ("return", "space"):
                label = self._picker[self._picker_sel]
                self.emit.info(f"gh-triage: picker selected {label!r}")
                self._picker = None
                self._toggle_label(label)
            return

        # triage view
        if key in ("n", "right"):
            self._advance()
        elif key in ("b", "left"):
            self._go_back()
        elif key == "tab":
            self._section = "available" if self._section == "current" else "current"
            self._label_sel = 0
            self.emit.schedule_render()
        elif key in ("j", "down"):
            issue = self._current()
            if issue:
                lst = issue.labels if self._section == "current" else self._available_labels(issue)[:18]
                self._label_sel = min(self._label_sel + 1, max(0, len(lst) - 1))
                self.emit.schedule_render()
        elif key in ("k", "up"):
            self._label_sel = max(0, self._label_sel - 1)
            self.emit.schedule_render()
        elif key in ("return", "space"):
            issue = self._current()
            if issue:
                lst = issue.labels if self._section == "current" else self._available_labels(issue)[:18]
                if 0 <= self._label_sel < len(lst):
                    self._toggle_label(lst[self._label_sel])
        elif key == "p":
            self.emit.info("gh-triage: open priority picker")
            self._picker     = PRIORITY_LABELS[:]
            self._picker_sel = 0
            self.emit.schedule_render()
        elif key == "l":
            self.emit.info("gh-triage: open load picker")
            self._picker     = LOAD_LABELS[:]
            self._picker_sel = 0
            self.emit.schedule_render()
        elif key == "a":
            area_labels = [l for l in self._all_labels if l.lower().startswith(AREA_PFX)]
            self.emit.info(f"gh-triage: open area picker count={len(area_labels)}")
            self._picker     = area_labels or ["(no area labels found)"]
            self._picker_sel = 0
            self.emit.schedule_render()
        elif key == "o":
            issue = self._current()
            if issue:
                self.emit.info(f"gh-triage: open #{issue.number} in browser")
                _gh("issue", "view", str(issue.number), "--web", cwd=self._root or None)
        elif key == "r":
            self.emit.info("gh-triage: refresh")
            asyncio.get_event_loop().create_task(self._load())
        elif key == "s":
            # skip — same as next but doesn't count as triaged
            self._history.append(self._idx)
            self._idx += 1
            self._label_sel = 0
            self._section = "current"
            self._picker = None
            self._flash = ""
            self._scroll_y = 0.0
            if self._idx >= len(self._issues):
                self._state = "done"
            self.emit.info(f"gh-triage: skip to idx={self._idx}")
            self.emit.schedule_render()

    def on_scroll(self, _ctx: RenderContext, id: str, offset_y: float) -> None:
        if id == "body":
            self._scroll_y = offset_y

    def on_click(self, ctx: RenderContext, x: float, y: float, button: str) -> None:
        if self._state != "triage" or button != "primary":
            return

        issue = self._current()
        if issue is None:
            return

        # picker click
        if self._picker is not None:
            opts = self._picker
            pw   = min(280.0, ctx.w - PAD * 4)
            ph   = PAD_TIGHT + PICKER_ROW * len(opts) + PAD_TIGHT
            px   = (ctx.w - pw) / 2
            py   = HEADER_H + PAD + ISSUE_HDR_H
            if px <= x <= px + pw and py <= y <= py + ph:
                i = int((y - py - PAD_TIGHT) / PICKER_ROW)
                if 0 <= i < len(opts):
                    self._picker_sel = i
                    label = opts[i]
                    self.emit.info(f"gh-triage: picker click {label!r}")
                    self._picker = None
                    self._toggle_label(label)
            else:
                self._picker = None
                self.emit.schedule_render()
            return

        cur_chips_y, avail_chips_y = self._label_section_y(ctx)

        # current labels row
        for rx, ry, rw, rh, label in self._chip_rects(cur_chips_y, issue.labels):
            if rx <= x <= rx + rw and ry <= y <= ry + rh:
                self.emit.info(f"gh-triage: click remove label={label!r}")
                self._toggle_label(label)
                return

        # available labels row
        for rx, ry, rw, rh, label in self._chip_rects(avail_chips_y, self._available_labels(issue)[:18]):
            if rx <= x <= rx + rw and ry <= y <= ry + rh:
                self.emit.info(f"gh-triage: click add label={label!r}")
                self._toggle_label(label)
                return


GhTriage().run()
