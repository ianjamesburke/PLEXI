#!/usr/bin/env python3
"""Mind Map — keyboard-driven brainstorming canvas for PGAP v3.

Vim-style navigation (h/j/k/l), Tab to add child, Enter to add sibling,
r/F2 to rename, Backspace/Delete to remove. Zoom with +/-/0. Click to select.
"""

import sys
import os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../../sdk/python'))

from dataclasses import dataclass, field
from plexi_sdk import App, RenderContext

# ── Layout constants ──────────────────────────────────────────────────────────
NODE_H     = 44.0
GAP        = 16.0
H_SPACING  = 210.0
NODE_W_MIN = 120.0
NODE_W_MAX = 160.0
ZOOM_MIN   = 0.4
ZOOM_MAX   = 2.0

# ── Color palette ─────────────────────────────────────────────────────────────
BG              = "#0d0d13"
NODE_BG         = "#16161f"
NODE_BORDER     = "#2a2a3e"
NODE_SEL_BG     = "#1e1b3a"
NODE_SEL_BORDER = "#7c6af7"
NODE_ROOT_BG    = "#1a183a"
EDGE            = "#252535"
TEXT            = "#cdd6f4"
TEXT_DIM        = "#6c7086"
ACCENT          = "#7c6af7"
STATUS_BG       = "#11111b"


@dataclass
class Node:
    id: int
    text: str
    parent: int | None       # None = root
    children: list[int] = field(default_factory=list)


class MindMapApp(App):

    def on_init(self, ctx: RenderContext) -> None:
        self._next_id = 0
        self._nodes: dict[int, Node] = {}
        self._root: int = self._new_node("Project Idea", parent=None)

        # Build default map
        problem   = self._new_node("Problem",    parent=self._root)
        _pain     = self._new_node("Current pain", parent=problem)
        _solution = self._new_node("Solution",   parent=self._root)
        _next     = self._new_node("Next step",  parent=self._root)

        self._selected: int = self._root
        self._pan_x: float = 0.0
        self._pan_y: float = 0.0
        self._zoom: float  = 1.0

        # Positions computed by _layout, keyed by node id
        self._positions: dict[int, tuple[float, float]] = {}

        # Edit-mode state
        self._editing: bool = False
        self._edit_original: str = ""   # text before edit started

        self.emit.status_summary("Mind Map ready")

    # ── Node helpers ──────────────────────────────────────────────────────────

    def _new_node(self, text: str, parent: int | None) -> int:
        nid = self._next_id
        self._next_id += 1
        self._nodes[nid] = Node(id=nid, text=text, parent=parent)
        if parent is not None:
            self._nodes[parent].children.append(nid)
        return nid

    def _delete_subtree(self, nid: int) -> None:
        """Remove node and all descendants; unlink from parent's children list."""
        node = self._nodes[nid]
        for cid in list(node.children):
            self._delete_subtree(cid)
        if node.parent is not None:
            parent = self._nodes[node.parent]
            parent.children = [c for c in parent.children if c != nid]
        del self._nodes[nid]

    # ── Layout ────────────────────────────────────────────────────────────────

    def _subtree_h(self, nid: int) -> float:
        node = self._nodes[nid]
        if not node.children:
            return NODE_H + GAP
        return sum(self._subtree_h(c) for c in node.children)

    def _layout(self, nid: int, x: float, y: float) -> dict[int, tuple[float, float]]:
        pos: dict[int, tuple[float, float]] = {nid: (x, y)}
        node = self._nodes[nid]
        if not node.children:
            return pos
        total = sum(self._subtree_h(c) for c in node.children)
        cy = y - total / 2
        for cid in node.children:
            ch = self._subtree_h(cid)
            pos.update(self._layout(cid, x + H_SPACING, cy + ch / 2))
            cy += ch
        return pos

    def _recompute_layout(self) -> None:
        self._positions = self._layout(self._root, 0.0, 0.0)

    # ── Pan/zoom helpers ──────────────────────────────────────────────────────

    def _center_on(self, nid: int, w: float, h: float) -> None:
        """Shift pan so the selected node appears at canvas center."""
        if nid not in self._positions:
            return
        nx, ny = self._positions[nid]
        # screen_x = nx * zoom + w/2 + pan_x  → we want screen_x = w/2
        self._pan_x = -nx * self._zoom
        self._pan_y = -ny * self._zoom

    def _screen_pos(self, nid: int, w: float, h: float) -> tuple[float, float]:
        nx, ny = self._positions.get(nid, (0.0, 0.0))
        sx = nx * self._zoom + w / 2 + self._pan_x
        sy = ny * self._zoom + h / 2 + self._pan_y
        return sx, sy

    def _node_w(self, node: Node) -> float:
        est = max(NODE_W_MIN, min(NODE_W_MAX, len(node.text) * 8.5 + 20))
        return est

    # ── Keyboard ──────────────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if self._editing:
            # Only Escape is handled; text_input widget owns everything else
            if key == "Escape":
                # Restore original text and exit edit mode
                self._nodes[self._selected].text = self._edit_original
                self._editing = False
                self._recompute_layout()
            return

        node = self._nodes[self._selected]

        # Navigation
        if key in ("h", "left"):
            if node.parent is not None:
                self._selected = node.parent
                self._center_on(self._selected, ctx.w, ctx.h)

        elif key in ("j", "down"):
            if node.parent is not None:
                siblings = self._nodes[node.parent].children
                idx = siblings.index(self._selected)
                if idx + 1 < len(siblings):
                    self._selected = siblings[idx + 1]
                    self._center_on(self._selected, ctx.w, ctx.h)

        elif key in ("k", "up"):
            if node.parent is not None:
                siblings = self._nodes[node.parent].children
                idx = siblings.index(self._selected)
                if idx > 0:
                    self._selected = siblings[idx - 1]
                    self._center_on(self._selected, ctx.w, ctx.h)

        elif key in ("l", "right"):
            if node.children:
                self._selected = node.children[0]
                self._center_on(self._selected, ctx.w, ctx.h)
            else:
                # No children — add one (same as Tab)
                self._add_child(ctx)

        elif key == "Tab":
            self._add_child(ctx)

        elif key == "Enter":
            self._add_sibling(ctx)

        elif key in ("r", "F2"):
            self._start_edit()

        elif key in ("Backspace", "Delete"):
            if self._selected != self._root:
                # Non-root nodes always have a parent; assert to narrow the type.
                assert node.parent is not None
                parent_id: int = node.parent
                # Find a reasonable node to select after deletion
                siblings = self._nodes[parent_id].children
                idx = siblings.index(self._selected)
                self._delete_subtree(self._selected)
                self._recompute_layout()
                # Select previous sibling, next sibling, or parent
                new_siblings = self._nodes[parent_id].children
                if new_siblings:
                    self._selected = new_siblings[max(0, idx - 1)]
                else:
                    self._selected = parent_id
                self._center_on(self._selected, ctx.w, ctx.h)

        elif key in ("+", "="):
            self._zoom = min(ZOOM_MAX, self._zoom * 1.15)

        elif key == "-":
            self._zoom = max(ZOOM_MIN, self._zoom / 1.15)

        elif key == "0":
            self._zoom = 1.0
            self._center_on(self._selected, ctx.w, ctx.h)

    def _add_child(self, ctx: RenderContext) -> None:
        cid = self._new_node("", parent=self._selected)
        self._selected = cid
        self._recompute_layout()
        self._center_on(cid, ctx.w, ctx.h)
        self._start_edit()

    def _add_sibling(self, ctx: RenderContext) -> None:
        node = self._nodes[self._selected]
        parent_id = node.parent if node.parent is not None else self._root
        # Insert after current node in parent's children list
        parent = self._nodes[parent_id]
        if self._selected in parent.children:
            idx = parent.children.index(self._selected)
        else:
            idx = len(parent.children) - 1
        sid = self._new_node("", parent=parent_id)
        # Move new node right after current node
        parent.children.remove(sid)
        parent.children.insert(idx + 1, sid)
        self._selected = sid
        self._recompute_layout()
        self._center_on(sid, ctx.w, ctx.h)
        self._start_edit()

    def _start_edit(self) -> None:
        self._edit_original = self._nodes[self._selected].text
        self._editing = True

    # ── Click ─────────────────────────────────────────────────────────────────

    def on_click(self, ctx: RenderContext, x: float, y: float, button: str) -> None:
        if self._editing:
            return
        margin = 8.0
        w, h = ctx.w, ctx.h
        for nid, node in self._nodes.items():
            sx, sy = self._screen_pos(nid, w, h)
            nw = self._node_w(node)
            nx0 = sx - nw / 2 - margin
            ny0 = sy - NODE_H / 2 - margin
            nx1 = sx + nw / 2 + margin
            ny1 = sy + NODE_H / 2 + margin
            if nx0 <= x <= nx1 and ny0 <= y <= ny1:
                self._selected = nid
                self._center_on(nid, w, h)
                return

    # ── Render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        self._recompute_layout()
        w, h = ctx.w, ctx.h
        STATUS_H = 28.0

        # Background
        ctx.rect(0, 0, w, h, fill=BG)

        canvas_h = h - STATUS_H

        # ── Draw edges (lines first, behind nodes) ────────────────────────────
        for nid, node in self._nodes.items():
            if node.parent is None:
                continue
            px, py = self._screen_pos(node.parent, w, canvas_h)
            cx, cy = self._screen_pos(nid, w, canvas_h)
            parent_node = self._nodes[node.parent]
            pw = self._node_w(parent_node)
            nw = self._node_w(node)
            # Line from right edge of parent to left edge of child
            x1 = px + pw / 2
            y1 = py
            x2 = cx - nw / 2
            y2 = cy
            # Draw as two segments with a horizontal midpoint
            mid_x = (x1 + x2) / 2
            ctx.line(x1, y1, mid_x, y1, color=EDGE, width=1.5)
            ctx.line(mid_x, y1, mid_x, y2, color=EDGE, width=1.5)
            ctx.line(mid_x, y2, x2, y2, color=EDGE, width=1.5)

        # ── Draw nodes ────────────────────────────────────────────────────────
        for nid, node in self._nodes.items():
            sx, sy = self._screen_pos(nid, w, canvas_h)
            nw = self._node_w(node)
            is_selected = nid == self._selected
            is_root = node.parent is None

            # Choose fill/border
            if is_selected:
                fill = NODE_SEL_BG
                border = NODE_SEL_BORDER
                border_w = 2.0
            elif is_root:
                fill = NODE_ROOT_BG
                border = NODE_BORDER
                border_w = 1.5
            else:
                fill = NODE_BG
                border = NODE_BORDER
                border_w = 1.0

            rx = sx - nw / 2
            ry = sy - NODE_H / 2

            # Border (slightly larger rect behind)
            ctx.rect(rx - border_w, ry - border_w,
                     nw + border_w * 2, NODE_H + border_w * 2,
                     fill=border, radius=8.0)
            # Fill
            ctx.rect(rx, ry, nw, NODE_H, fill=fill, radius=7.0)

            # Label — show text_input widget when editing this node
            if is_selected and self._editing:
                submitted = ctx.text_input(
                    "node-edit",
                    x=rx + 8,
                    y=ry + (NODE_H - 20) / 2,
                    w=nw - 16,
                    placeholder="Node label…",
                )
                if submitted is not None:
                    self._nodes[nid].text = submitted
                    self._editing = False
                    self._recompute_layout()
            else:
                label = node.text if node.text else "(empty)"
                text_color = TEXT if node.text else TEXT_DIM
                text_size = 14.0 if not is_root else 15.0
                bold = is_root
                # Truncate to fit node width (approx 7.5px per char at 14pt)
                chars = int((nw - 16) / 7.5)
                display = label if len(label) <= chars else label[:chars - 1] + "…"
                ctx.text(rx + 8, ry + (NODE_H - text_size) / 2,
                         display, size=text_size, color=text_color, bold=bold)

        # ── Status bar ────────────────────────────────────────────────────────
        sel_node = self._nodes[self._selected]
        sel_text = sel_node.text or "(empty)"
        node_count = len(self._nodes)
        status = (f"{sel_text}  ·  {node_count} nodes  ·  "
                  "Tab child  ·  Enter sibling  ·  h/j/k/l navigate  ·  r rename  ·  Del delete")
        ctx.rect(0, canvas_h, w, STATUS_H, fill=STATUS_BG)
        ctx.text(12, canvas_h + (STATUS_H - 12) / 2, status, size=12.0, color=TEXT_DIM)


if __name__ == "__main__":
    MindMapApp().run()
