"""Plexi SDK — Declarative Host-Rendered UI Component Library (`plexi_sdk.ui`).

All components in this module serialize to a declarative ``UiNode`` tree via
``to_node()`` and are rendered natively by the Plexi host.

Usage::

    from plexi_sdk import theme
    from plexi_sdk.ui import (
        AppBar, Badge, Button, Card, Column, DataTable, Divider,
        FooterKeys, HStack, Heading, Label, Pending, Scrollable,
        Section, SelectList, Skeleton, Spacer, Text, TextInput,
        SPACE_SM, SPACE_MD,
    )
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Any, Literal

from ._keys import normalize_key

# ── Design Tokens ─────────────────────────────────────────────────────────────

SPACE_XS: int = 4
SPACE_SM: int = 8
SPACE_MD: int = 12
SPACE_LG: int = 16
SPACE_XL: int = 24

TEXT_HINT: int = 10
TEXT_CAPTION: int = 11
TEXT_BODY: int = 13
TEXT_HEADING: int = 15
TEXT_TITLE: int = 18
TEXT_TITLE_XL: int = 22

RADIUS_SM: int = 3
RADIUS_MD: int = 6
RADIUS_LG: int = 8
RADIUS_BADGE: int = 10

# Semantic color roles accepted by Badge / Banner (`decode_badge_color` in
# `src/host/wasm_python.rs`).
BadgeColor = Literal[
    "accent", "success", "warning", "danger", "neutral",
    "red", "green", "yellow",
]
BADGE_COLORS: tuple[str, ...] = (
    "accent", "success", "warning", "danger", "neutral",
    "red", "green", "yellow",
)

# Semantic color roles accepted by Text / Label / Heading (Badge roles + "muted"),
# in addition to `#rrggbb` / `#rrggbbaa` hex strings from `plexi_sdk.theme`.
TextColor = Literal[
    "accent", "success", "warning", "danger", "neutral", "muted",
    "red", "green", "yellow",
]
TEXT_COLORS: tuple[str, ...] = BADGE_COLORS + ("muted",)


def _validate_badge_color(color: str, *, widget: str = "Badge") -> str:
    if color not in BADGE_COLORS:
        valid = ", ".join(repr(c) for c in BADGE_COLORS[:5])
        raise ValueError(
            f"{widget} color must be one of {valid} "
            f"(or theme alias 'red'/'green'/'yellow'), got {color!r}. "
            "Badge colors are semantic theme roles, not literal CSS colors."
        )
    return color


def _validate_text_color(color: str | None, *, widget: str = "Text") -> str:
    if color is None or color == "":
        return ""
    if not isinstance(color, str):
        raise TypeError(f"{widget} color must be a str or None, got {type(color).__name__}")
    if color in TEXT_COLORS or color.startswith("#"):
        return color
    valid = ", ".join(repr(c) for c in ("accent", "success", "warning", "danger", "neutral", "muted"))
    raise ValueError(
        f"{widget} color must be a semantic role ({valid}, or 'red'/'green'/'yellow') "
        f"or a hex token from plexi_sdk.theme (e.g. theme.red, theme.text_muted), got {color!r}."
    )


# ── Scroll & Cursor Helpers ───────────────────────────────────────────────────


def ensure_visible(
    scroll_offset: float,
    viewport_h: float,
    item_top: float,
    item_bottom: float,
    margin: float = 0.0,
) -> float:
    """Return a clamped scroll offset that keeps ``[item_top, item_bottom]`` visible."""
    if viewport_h <= 0:
        return scroll_offset
    lo = item_top - margin
    hi = item_bottom + margin
    if lo < scroll_offset:
        return max(0.0, lo)
    if hi > scroll_offset + viewport_h:
        return max(0.0, hi - viewport_h)
    return scroll_offset


def move_cursor(event_or_key: Any, count: int, selected: int = 0) -> int | None:
    """Compute the next 0-based cursor index for standard list navigation keys.

    Handles ``j`` / ``down``, ``k`` / ``up``, ``g`` / ``home``, and ``G`` / ``end``.
    Returns the new clamped index in ``0..count-1``, or ``None`` if ``event_or_key``
    is not a list navigation key or ``count <= 0``.
    """
    if count <= 0:
        return None
    raw_key = getattr(event_or_key, "key", event_or_key)
    if not isinstance(raw_key, str):
        return None
    cur = max(0, min(int(selected), count - 1))
    if raw_key in ("j", "down", "Down"):
        return min(count - 1, cur + 1)
    if raw_key in ("k", "up", "Up"):
        return max(0, cur - 1)
    if raw_key in ("g", "home", "Home"):
        return 0
    if raw_key in ("G", "end", "End"):
        return count - 1
    return None


# ── Component Base ────────────────────────────────────────────────────────────


class Component:
    """Base class for all declarative UI components.

    Every concrete component implements :meth:`to_node` returning a JSON-serializable
    dictionary consumed by the host UI decoder.
    """

    def to_node(self) -> dict | None:
        return None


class Keyed(Component):
    """Give a component a stable explicit key for lists and local state."""

    def __init__(self, child: Component, *, key: str) -> None:
        if not isinstance(child, Component):
            raise TypeError(f"Keyed child must subclass Component, got {type(child).__name__}")
        if not isinstance(key, str) or not key:
            raise ValueError("Keyed key must be a non-empty string")
        self.child = child
        self.key = key

    def to_node(self) -> dict | None:
        return self.child.to_node()


def _validate_component_list(items: Any, container_name: str) -> None:
    if not isinstance(items, list):
        raise TypeError(
            f"{container_name} children must be a list of Component instances, "
            f"got {type(items).__name__}"
        )
    for i, item in enumerate(items):
        if not isinstance(item, Component):
            raise TypeError(
                f"{container_name} children[{i}] must subclass Component, "
                f"got {type(item).__name__}: {item!r}"
            )


# ── Layout Containers ─────────────────────────────────────────────────────────


class Column(Component):
    """Vertical stack of child components."""

    def __init__(
        self,
        children: list[Component] | None = None,
        gap: float | None = None,
        padding: float | tuple[float, float] | tuple[float, float, float, float] = 0.0,
        *,
        align: str = "start",
        grow: bool = False,
    ) -> None:
        items = list(children) if children is not None else []
        _validate_component_list(items, "Column")
        self.children = items
        self.gap: float | None = float(gap) if gap is not None else None
        self.padding = padding
        self.align = align
        self.grow = bool(grow)

    def to_node(self) -> dict | None:
        children = []
        for c in self.children:
            n = c.to_node()
            if n is None:
                return None
            children.append(n)
        out: dict[str, Any] = {
            "type": "column",
            "children": children,
            "align": self.align,
            "grow": self.grow,
        }
        if self.gap is not None:
            out["gap"] = self.gap
        return out


class HStack(Component):
    """Horizontal stack of child components (`Row` alias)."""

    def __init__(
        self,
        children: list[Component] | None = None,
        gap: float | None = None,
        *,
        align: str = "start",
        grow: bool = False,
    ) -> None:
        items = list(children) if children is not None else []
        _validate_component_list(items, "HStack")
        self.children = items
        self.gap: float | None = float(gap) if gap is not None else None
        self.align = align
        self.grow = bool(grow)

    def to_node(self) -> dict | None:
        children = []
        for c in self.children:
            n = c.to_node()
            if n is None:
                return None
            children.append(n)
        out: dict[str, Any] = {
            "type": "row",
            "children": children,
            "align": self.align,
            "grow": self.grow,
        }
        if self.gap is not None:
            out["gap"] = self.gap
        return out


Row = HStack


class Card(Component):
    """Elevated surface container with rounded border and inner padding."""

    def __init__(
        self,
        children: list[Component] | None = None,
        padding: float = SPACE_MD,
        gap: float | None = SPACE_SM,
        *,
        bg_color: str | None = None,
        border_color: str | None = None,
        radius: float = RADIUS_MD,
    ) -> None:
        items = list(children) if children is not None else []
        _validate_component_list(items, "Card")
        self.children = items
        self.padding = float(padding)
        self.gap: float | None = float(gap) if gap is not None else None
        self.bg_color = bg_color
        self.border_color = border_color
        self.radius = float(radius)

    def to_node(self) -> dict | None:
        children = []
        for c in self.children:
            n = c.to_node()
            if n is None:
                return None
            children.append(n)
        out: dict[str, Any] = {
            "type": "card",
            "padding": self.padding,
            "children": children,
        }
        if self.gap is not None:
            out["gap"] = self.gap
        return out


class Scrollable(Component):
    """Scrollable viewport wrapping a single child component."""

    def __init__(
        self,
        child: Component,
        *,
        horizontal: bool = False,
    ) -> None:
        if not isinstance(child, Component):
            raise TypeError(
                f"Scrollable child must subclass Component, got {type(child).__name__}: {child!r}"
            )
        self.child = child
        self.horizontal = bool(horizontal)
        self.scroll_offset: float = 0.0

    def to_node(self) -> dict | None:
        child_node = self.child.to_node()
        if child_node is None:
            return None
        return {
            "type": "scroll",
            "child": child_node,
            "horizontal": self.horizontal,
        }


class Grid(Component):
    """Multi-column grid layout composed onto rows and columns."""

    def __init__(
        self,
        columns: int,
        children: list[Component] | None = None,
        gap: float = SPACE_SM,
    ) -> None:
        self.columns = max(1, int(columns))
        self.children = list(children) if children is not None else []
        self.gap = float(gap)

    def to_node(self) -> dict:
        child_nodes = [
            c.to_node() if hasattr(c, "to_node") else c
            for c in self.children
        ]
        rows = []
        for i in range(0, len(child_nodes), self.columns):
            chunk = child_nodes[i : i + self.columns]
            rows.append({
                "type": "row",
                "children": chunk,
                "gap": self.gap,
            })
        return {
            "type": "column",
            "children": rows,
            "gap": self.gap,
        }


# ── Chrome & Typography ───────────────────────────────────────────────────────


class AppBar(Component):
    """Top application header bar with title and optional subtitle."""

    BAND_H: float = 34.0
    DIVIDER_H: float = 1.0
    TITLE_SIZE: float = 16.0

    def __init__(
        self,
        title: str,
        subtitle: str = "",
        *,
        accent: str | None = None,
    ) -> None:
        self.title = title
        self.subtitle = subtitle
        self.accent = accent

    def to_node(self) -> dict:
        return {
            "type": "app_bar",
            "title": self.title,
            "subtitle": self.subtitle,
        }


class Section(Component):
    """Titled section header with a divider, optionally wrapping body children."""

    def __init__(
        self,
        title: str,
        children: list[Component] | Component | None = None,
        *more_children: Component,
        gap: float = SPACE_SM,
    ) -> None:
        self.title = title
        self.gap = float(gap)
        collected: list[Component] = []
        if isinstance(children, list):
            collected.extend(children)
        elif children is not None:
            collected.append(children)
        if more_children:
            collected.extend(more_children)
        _validate_component_list(collected, "Section")
        self.children = collected

    def to_node(self) -> dict | None:
        nodes: list[dict] = [
            {"type": "text", "text": self.title, "bold": True},
            {"type": "divider"},
        ]
        for child in self.children:
            child_node = child.to_node()
            if child_node is None:
                return None
            nodes.append(child_node)
        return {
            "type": "column",
            "children": nodes,
            "gap": self.gap if self.children else 4.0,
        }


class Heading(Component):
    """Bold section or view heading."""

    def __init__(
        self,
        text: str = "",
        size: float = TEXT_HEADING,
        color: str | None = None,
        *,
        align: str = "left",
        truncate: bool = False,
    ) -> None:
        self.text = text
        self.size = float(size)
        self.color = _validate_text_color(color, widget="Heading") or None
        self.align = align
        self.truncate = bool(truncate)

    def to_node(self) -> dict:
        node: dict[str, Any] = {
            "type": "text",
            "text": self.text,
            "size": self.size,
            "bold": True,
            "truncate": self.truncate,
            "align": self.align,
        }
        if self.color:
            node["color"] = self.color
        return node


class Label(Component):
    """Single-line or body text label with optional semantic ``tone`` or ``color``."""

    def __init__(
        self,
        text: str = "",
        size: float | None = None,
        color: str | None = None,
        bold: bool = False,
        elide: bool = False,
        *,
        tone: str = "body",
        align: str = "left",
        truncate: bool = False,
    ) -> None:
        self.text = text
        self.size = float(size) if size is not None else None
        self.color = _validate_text_color(color, widget="Label") or None
        self.bold = bool(bold)
        self.elide = bool(elide or truncate)
        self.tone = tone
        self.align = align

    def to_node(self) -> dict:
        node: dict[str, Any] = {
            "type": "label",
            "text": self.text,
            "tone": self.tone,
            "size": self.size,
            "bold": self.bold,
            "truncate": self.elide,
            "align": self.align,
        }
        if self.color:
            node["color"] = self.color
        elif self.tone == "hint":
            node["color"] = "muted"
        return node


@dataclass
class Text(Component):
    """Declarative text primitive with font size, styling, color, truncation, and alignment."""

    text: str = ""
    size: float = 13.0
    color: str = ""
    bold: bool = False
    italic: bool = False
    monospace: bool = False
    truncate: bool = False
    align: str = "left"
    tone: str = ""

    def __post_init__(self) -> None:
        self.color = _validate_text_color(self.color, widget="Text")
        if not self.color and self.tone == "hint":
            self.color = "muted"
        if self.align not in ("left", "start", "center", "right", "end"):
            raise ValueError(
                f"Text align must be 'left', 'start', 'center', 'right', or 'end', got {self.align!r}"
            )

    def to_node(self) -> dict:
        node: dict[str, Any] = {
            "type": "text",
            "text": self.text,
            "size": self.size,
            "bold": self.bold,
            "italic": self.italic,
            "monospace": self.monospace,
            "truncate": self.truncate,
            "align": self.align,
        }
        if self.color:
            node["color"] = self.color
        if self.tone:
            node["tone"] = self.tone
        return node


@dataclass
class Spacer(Component):
    """Fixed or flex gap. ``grow=True`` expands to consume remaining space."""

    size: float = SPACE_MD
    grow: bool = False
    flex: bool = False

    def to_node(self) -> dict:
        return {"type": "spacer", "size": float(self.size), "grow": bool(self.grow or self.flex)}


class Divider(Component):
    """Horizontal rule separating vertical sections."""

    def __init__(self, color: str | None = None) -> None:
        self.color = color

    def to_node(self) -> dict:
        return {"type": "divider"}


@dataclass
class Badge(Component):
    """Semantic status pill (`accent`, `success`, `warning`, `danger`, `neutral`)."""

    label: str = ""
    color: BadgeColor = "neutral"
    text: str = ""
    tone: BadgeColor | None = None

    def __post_init__(self) -> None:
        if self.text and not self.label:
            self.label = self.text
        elif self.label and not self.text:
            self.text = self.label
        if self.tone is not None:
            self.color = self.tone
        _validate_badge_color(self.color, widget="Badge")

    def to_node(self) -> dict:
        return {
            "type": "badge",
            "text": self.label,
            "label": self.label,
            "color": self.color,
        }


@dataclass
class FooterKeys(Component):
    """Bottom-pinned keybinding hint bar."""

    shortcuts: list[tuple[Any, str]] = field(default_factory=list)
    divider: bool = True

    def __post_init__(self) -> None:
        for i, item in enumerate(self.shortcuts):
            if not isinstance(item, (tuple, list)) or len(item) != 2:
                raise TypeError(
                    f"FooterKeys shortcuts[{i}] must be a (key_or_keys, description) tuple, "
                    f"got {type(item).__name__}: {item!r}. "
                    f"Example: FooterKeys([(\"j\", \"down\"), ([\"g\", \"G\"], \"ends\")])"
                )

    def to_node(self) -> dict:
        entries = []
        for keys_or_key, desc in self.shortcuts:
            if isinstance(keys_or_key, str):
                keys = [keys_or_key]
            else:
                keys = list(keys_or_key)
            entries.append({"keys": keys, "description": desc})
        inner = {"type": "footer_keys", "entries": entries, "divider": self.divider}
        return {"type": "pinned", "edge": "bottom", "child": inner}


# ── Buttons & Form Controls ───────────────────────────────────────────────────


@dataclass
class Button(Component):
    """Interactive button dispatching ``UiAction(on_click)`` when activated."""

    label: str
    on_click: str = ""
    style: str = "secondary"
    disabled: bool = False

    def to_node(self) -> dict:
        return {
            "type": "button",
            "label": self.label,
            "on_click": self.on_click,
            "style": self.style,
            "disabled": self.disabled,
        }


class Actions(Component):
    """Horizontal row of ``Button`` components (`ActionBar` alias)."""

    def __init__(
        self,
        buttons: list[Button] | None = None,
        *,
        actions: list[Button] | None = None,
        gap: float = SPACE_SM,
        align: str = "start",
    ) -> None:
        items = actions if actions is not None else (buttons if buttons is not None else [])
        if not isinstance(items, list):
            raise TypeError(
                f"ActionBar actions must be a list of Button instances, got {type(items).__name__}"
            )
        for i, item in enumerate(items):
            if not isinstance(item, Button):
                raise TypeError(
                    f"ActionBar actions[{i}] must be a Button, got {type(item).__name__}"
                )
        self.buttons = list(items)
        self.actions = self.buttons
        self.gap = float(gap)
        self.align = align

    def to_node(self) -> dict:
        return {
            "type": "row",
            "children": [b.to_node() for b in self.buttons],
            "gap": self.gap,
            "align": self.align,
        }


ActionBar = Actions


class TextInput(Component):
    """Editable text input rendered natively by the host (`TextEdit` alias)."""

    def __init__(
        self,
        id: str = "",
        value: str = "",
        placeholder: str = "",
        on_change: str = "",
        on_submit: str = "",
        password: bool = False,
        *,
        autofocus: bool = False,
        node_id: str = "",
        multiline: bool = False,
        max_length: int | None = None,
        height: float = 0.0,
    ) -> None:
        resolved_id = id or node_id
        self.id = resolved_id
        self.node_id = resolved_id
        self.value = value
        self.placeholder = placeholder
        self.on_change = on_change or resolved_id
        self.on_submit = on_submit or resolved_id
        self.password = bool(password)
        self.autofocus = bool(autofocus)
        self.multiline = bool(multiline)
        self.max_length = max_length
        self.height = float(height)

    def to_node(self) -> dict:
        return {
            "type": "TextInput",
            "value": self.value,
            "placeholder": self.placeholder,
            "on_change": self.on_change or self.id,
            "on_submit": self.on_submit or self.id,
            "password": self.password,
            "autofocus": self.autofocus,
        }


TextEdit = TextInput


class FormField(Component):
    """Labeled ``TextInput`` column with optional required indicator."""

    def __init__(
        self,
        id: str,
        label: str,
        *,
        value: str = "",
        placeholder: str = "",
        required: bool = False,
        password: bool = False,
        autofocus: bool = False,
        on_change: str = "",
        on_submit: str = "",
    ) -> None:
        self.id = id
        self.label = label
        self.value = value
        self.placeholder = placeholder
        self.required = bool(required)
        self.password = bool(password)
        self.autofocus = bool(autofocus)
        self.on_change = on_change or id
        self.on_submit = on_submit or id

    def to_node(self) -> dict:
        label_text = f"{self.label} *" if self.required else self.label
        return {
            "type": "column",
            "children": [
                {"type": "text", "text": label_text, "bold": True},
                TextInput(
                    self.id,
                    value=self.value,
                    placeholder=self.placeholder,
                    on_change=self.on_change,
                    on_submit=self.on_submit,
                    password=self.password,
                    autofocus=self.autofocus,
                ).to_node(),
            ],
            "gap": 4.0,
        }


class Checkbox(Component):
    """Boolean checkbox control dispatching ``UiAction(node_id)`` on click."""

    def __init__(
        self,
        node_id: str,
        label: str = "",
        checked: bool = False,
        disabled: bool = False,
    ) -> None:
        self.node_id = node_id
        self.label = label
        self.checked = bool(checked)
        self.disabled = bool(disabled)

    def to_node(self) -> dict:
        mark = "[x]" if self.checked else "[ ]"
        text = f"{mark} {self.label}" if self.label else mark
        return {
            "type": "button",
            "label": text,
            "on_click": self.node_id,
            "style": "primary" if self.checked else "secondary",
            "disabled": self.disabled,
        }


class Toggle(Component):
    """Boolean toggle control (`Switch` alias)."""

    def __init__(
        self,
        node_id: str,
        value: bool | None = None,
        label: str = "",
        *,
        on: bool | None = None,
        disabled: bool = False,
    ) -> None:
        self.node_id = node_id
        resolved = on if on is not None else (value if value is not None else False)
        self.value = bool(resolved)
        self.on = self.value
        self.label = label
        self.disabled = bool(disabled)

    def to_node(self) -> dict:
        state_button = {
            "type": "button",
            "label": "on" if self.value else "off",
            "on_click": self.node_id,
            "style": "primary" if self.value else "secondary",
            "disabled": self.disabled,
        }
        if self.label:
            return {
                "type": "row",
                "children": [
                    {"type": "text", "text": self.label},
                    state_button,
                ],
                "gap": 8.0,
            }
        return state_button


Switch = Toggle


class Radio(Component):
    """Single-choice radio group dispatching ``UiAction(f"{node_id}:{idx}")``."""

    def __init__(
        self,
        node_id: str,
        options: list[str] | None = None,
        selected: int = 0,
        disabled: bool = False,
    ) -> None:
        self.node_id = node_id
        self.options = list(options) if options is not None else []
        self.selected = int(selected)
        self.disabled = bool(disabled)

    def to_node(self) -> dict:
        children = [
            {
                "type": "button",
                "label": f"({'*' if i == self.selected else ' '}) {opt}",
                "on_click": f"{self.node_id}:{i}",
                "style": "primary" if i == self.selected else "secondary",
                "disabled": self.disabled,
            }
            for i, opt in enumerate(self.options)
        ]
        return {"type": "column", "children": children, "gap": 4.0}


class Slider(Component):
    """Numeric range slider control with decrement/increment buttons and progress bar."""

    def __init__(
        self,
        node_id: str,
        value: float = 0.0,
        min: float = 0.0,
        max: float = 1.0,
        step: float = 0.01,
        disabled: bool = False,
    ) -> None:
        self.node_id = node_id
        self.value = float(value)
        self.min = float(min)
        self.max = float(max)
        self.step = float(step)
        self.disabled = bool(disabled)

    def to_node(self) -> dict:
        span = self.max - self.min
        ratio = (self.value - self.min) / span if span > 0 else 0.0
        ratio = max(0.0, min(1.0, ratio))
        return {
            "type": "row",
            "children": [
                {
                    "type": "button",
                    "label": "-",
                    "on_click": f"{self.node_id}:dec",
                    "style": "secondary",
                    "disabled": self.disabled,
                },
                {"type": "progress-bar", "value": ratio, "max": 1.0},
                {
                    "type": "button",
                    "label": "+",
                    "on_click": f"{self.node_id}:inc",
                    "style": "secondary",
                    "disabled": self.disabled,
                },
                {"type": "text", "text": f"{self.value:.2f}"},
            ],
            "gap": 6.0,
        }


class Select(Component):
    """Single-choice option selector."""

    def __init__(
        self,
        node_id: str,
        options: list[str] | None = None,
        selected: int | None = None,
        placeholder: str = "Select…",
        disabled: bool = False,
    ) -> None:
        self.node_id = node_id
        self.options = list(options) if options is not None else []
        self.selected = selected
        self.placeholder = placeholder
        self.disabled = bool(disabled)

    def to_node(self) -> dict:
        if not self.options:
            return {
                "type": "button",
                "label": self.placeholder,
                "on_click": self.node_id,
                "style": "secondary",
                "disabled": True,
            }
        children = [
            {
                "type": "button",
                "label": opt,
                "on_click": f"{self.node_id}:{i}",
                "style": "primary" if i == self.selected else "secondary",
                "disabled": self.disabled,
            }
            for i, opt in enumerate(self.options)
        ]
        return {"type": "row", "children": children, "gap": 4.0}


class DateTimePicker(Component):
    """Date/time input field."""

    def __init__(
        self,
        node_id: str,
        value: str = "",
        mode: str = "date",
        disabled: bool = False,
    ) -> None:
        self.node_id = node_id
        self.value = value
        self.mode = mode
        self.disabled = bool(disabled)

    def to_node(self) -> dict:
        placeholder = {
            "date": "YYYY-MM-DD",
            "time": "HH:MM",
            "datetime": "YYYY-MM-DD HH:MM",
        }.get(self.mode, "YYYY-MM-DD")
        return {
            "type": "TextInput",
            "value": self.value,
            "placeholder": placeholder,
            "on_change": self.node_id,
            "on_submit": self.node_id,
            "password": False,
        }


# ── Lists, Navigation & Tables ────────────────────────────────────────────────


class SelectList(Component):
    """Scrollable, selectable list view with keyboard and click navigation.

    ``items`` may be:
    - ``list[dict]`` with ``name`` (or ``label``) and optional ``description``,
      ``leading``, ``trailing``
    - ``list[str]`` of row labels
    - ``list[Component]`` of custom row components

    When ``on_select`` is set, clicking row ``i`` dispatches
    ``UiAction(f"{on_select}:{i}")``. Use :meth:`SelectList.move_cursor` (or
    top-level :func:`move_cursor`) in ``update()`` to handle ``j``/``k``/arrow
    keys.
    """

    move_cursor = staticmethod(move_cursor)

    def __init__(
        self,
        items: list[Any],
        selected_idx: int = 0,
        scroll_height: float = 0,
        *,
        selected: int | None = None,
        on_select: str | None = None,
    ) -> None:
        if not isinstance(items, list):
            raise TypeError(
                f"SelectList items must be a list, got {type(items).__name__}."
            )
        for i, item in enumerate(items):
            if isinstance(item, (str, Component)):
                continue
            if isinstance(item, dict):
                if "name" not in item and "label" not in item and "type" not in item and "kind" not in item:
                    raise ValueError(
                        f"SelectList items[{i}] dict is missing required 'name' key. "
                        f"Got keys: {list(item.keys())}."
                    )
                continue
            raise TypeError(
                f"SelectList items[{i}] must be a dict, str, or Component, "
                f"got {type(item).__name__}: {item!r}"
            )
        self.items = items
        resolved_sel = selected if selected is not None else selected_idx
        self.selected_idx = int(resolved_sel) if resolved_sel is not None else 0
        self.scroll_height = float(scroll_height)
        self.on_select = on_select

    def handle_key(self, event_or_key: Any, count: int | None = None, selected: int = 0) -> Any:
        """Move selection for ``j``/``k``/``up``/``down``/``g``/``G``/``home``/``end``.

        When called on an instance as ``list_widget.handle_key(key)``, mutates
        ``self.selected_idx`` and returns ``True`` if the cursor moved.
        When called with ``(event_or_key, count, selected)``, acts like
        :func:`move_cursor` and returns the new index or ``None``.
        """
        if count is not None:
            return move_cursor(event_or_key, count, selected)
        nxt = move_cursor(event_or_key, len(self.items), self.selected_idx)
        if nxt is None or nxt == self.selected_idx:
            return False
        self.selected_idx = nxt
        return True

    def to_node(self) -> dict:
        return {
            "type": "select_list",
            "items": self.items,
            "selected_idx": self.selected_idx,
            "on_select": self.on_select,
        }


class Breadcrumb(Component):
    """Horizontal trail of path segments separated by ``/``."""

    def __init__(self, items: list[str] | None = None) -> None:
        self.items = list(items) if items is not None else []

    def to_node(self) -> dict:
        children: list[dict] = []
        for i, seg in enumerate(self.items):
            if i > 0:
                children.append({"type": "text", "text": "/"})
            children.append({
                "type": "text",
                "text": str(seg),
                "bold": i == len(self.items) - 1,
            })
        return {"type": "row", "children": children, "gap": 6.0}


class Tabs(Component):
    """Tabbed container or standalone tab bar (`TabBar` alias).

    Can be constructed either with ``(label, panel_component)`` pairs::

        Tabs([("Overview", overview_panel), ("Logs", logs_panel)], active=0, node_id="tabs")

    or as a standalone tab bar::

        TabBar("gallery-tabs", tabs=["Overview", "Details", "Logs"], active=0)
    """

    def __init__(
        self,
        *args: Any,
        tabs: list[Any] | None = None,
        active: int = 0,
        node_id: str = "tabs",
    ) -> None:
        raw_tabs: list[Any] = []
        resolved_active = active
        resolved_node_id = node_id

        if args:
            first = args[0]
            if isinstance(first, str):
                resolved_node_id = first
                if len(args) >= 2 and isinstance(args[1], list):
                    raw_tabs = list(args[1])
                    if len(args) >= 3:
                        resolved_active = int(args[2])
                elif tabs is not None:
                    raw_tabs = list(tabs)
                    if len(args) >= 2:
                        resolved_active = int(args[1])
            else:
                raw_tabs = list(first)
                if len(args) >= 2:
                    resolved_active = int(args[1])
                if len(args) >= 3:
                    resolved_node_id = str(args[2])
        elif tabs is not None:
            raw_tabs = list(tabs)

        self.node_id = resolved_node_id
        self.active = int(resolved_active)
        self.tabs: list[tuple[str, Component | None]] = [
            (str(t[0]), t[1]) if isinstance(t, tuple) else (str(t), None)
            for t in raw_tabs
        ]
        self._bar_only = bool(self.tabs) and all(panel is None for _, panel in self.tabs)

    def to_node(self) -> dict:
        tab_buttons = [
            {
                "type": "button",
                "label": label,
                "on_click": f"{self.node_id}:{i}",
                "style": "primary" if i == self.active else "secondary",
                "disabled": False,
            }
            for i, (label, _) in enumerate(self.tabs)
        ]
        tab_bar = {
            "type": "row",
            "children": tab_buttons,
            "gap": 4.0,
        }
        if self._bar_only:
            return tab_bar
        children: list[Any] = [tab_bar]
        if self.tabs:
            idx = max(0, min(self.active, len(self.tabs) - 1))
            active_panel = self.tabs[idx][1]
            if active_panel is not None:
                panel_node = active_panel.to_node() if hasattr(active_panel, "to_node") else active_panel
                if panel_node is not None:
                    children.append(panel_node)
        return {
            "type": "column",
            "children": children,
            "gap": 8.0,
        }


TabBar = Tabs


class Pagination(Component):
    """Prev / page-indicator / Next pagination bar."""

    def __init__(self, node_id: str, page: int = 0, total: int = 1) -> None:
        self.node_id = node_id
        self.page = int(page)
        self.total = max(1, int(total))

    def to_node(self) -> dict:
        return {
            "type": "row",
            "children": [
                {
                    "type": "button",
                    "label": "‹ Prev",
                    "on_click": f"{self.node_id}:prev",
                    "style": "secondary",
                    "disabled": self.page <= 0,
                },
                {
                    "type": "text",
                    "text": f"{self.page + 1} / {self.total}",
                },
                {
                    "type": "button",
                    "label": "Next ›",
                    "on_click": f"{self.node_id}:next",
                    "style": "secondary",
                    "disabled": self.page + 1 >= self.total,
                },
            ],
            "gap": 8.0,
        }


class DataTable(Component):
    """Structured multi-column table with header row, selection, and row-click action (`Table` alias).

    When ``on_select`` is set, clicking row ``i`` dispatches
    ``UiAction(f"{on_select}:{i}")``.
    """

    def __init__(
        self,
        columns: list[str] | None = None,
        rows: list[list[Any]] | None = None,
        *,
        selected: int | None = None,
        selected_idx: int | None = None,
        on_select: str | None = None,
    ) -> None:
        self.columns = [str(c) for c in (columns or [])]
        self.rows = [[str(cell) for cell in r] for r in (rows or [])]
        resolved_sel = selected if selected is not None else selected_idx
        self.selected = int(resolved_sel) if resolved_sel is not None else None
        self.on_select = on_select

    def to_node(self) -> dict:
        header = {
            "type": "row",
            "children": [
                {"type": "text", "text": str(col), "bold": True}
                for col in self.columns
            ],
            "gap": 12.0,
        }
        body = [
            {
                "type": "row",
                "children": [{"type": "text", "text": str(cell)} for cell in r],
                "gap": 12.0,
            }
            for r in self.rows
        ]
        return {
            "type": "DataTable",
            "columns": list(self.columns),
            "rows": [list(r) for r in self.rows],
            "selected": self.selected,
            "on_select": self.on_select,
            "children": [header, *body],
            "gap": 4.0,
        }


Table = DataTable


# ── Feedback & Status ─────────────────────────────────────────────────────────


class Banner(Component):
    """Status banner with semantic ``tone`` badge and message."""

    def __init__(
        self,
        text: str,
        tone: BadgeColor = "neutral",
        title: str = "",
        dismissible: bool = False,
        node_id: str = "",
    ) -> None:
        self.text = text
        self.tone: BadgeColor = _validate_badge_color(tone, widget="Banner")  # type: ignore[assignment]
        self.title = title
        self.dismissible = bool(dismissible)
        self.node_id = node_id

    def to_node(self) -> dict:
        badge_label = self.title or self.tone
        children: list[dict] = [
            {"type": "badge", "label": badge_label, "color": self.tone},
            {"type": "text", "text": self.text},
        ]
        if self.dismissible and self.node_id:
            children.append({
                "type": "button",
                "label": "×",
                "on_click": self.node_id,
                "style": "ghost",
                "disabled": False,
            })
        return {"type": "row", "children": children, "gap": 8.0}


class ProgressBar(Component):
    """Horizontal progress indicator (`Progress` alias)."""

    def __init__(
        self,
        value: float = 0.0,
        max_value: float = 1.0,
        *,
        label: str = "",
        indeterminate: bool = False,
    ) -> None:
        self.value = float(value)
        self.max_value = float(max_value) if max_value > 0 else 1.0
        self.label = label
        self.indeterminate = bool(indeterminate)

    def to_node(self) -> dict:
        if self.indeterminate:
            return {"type": "spinner", "label": self.label}
        bar = {
            "type": "progress-bar",
            "value": self.value,
            "max": self.max_value,
        }
        if self.label:
            return {
                "type": "column",
                "children": [
                    {"type": "text", "text": self.label},
                    bar,
                ],
                "gap": 4.0,
            }
        return bar


Progress = ProgressBar


@dataclass
class Spinner(Component):
    """Animated indeterminate loading spinner."""

    label: str = ""

    def to_node(self) -> dict:
        return {"type": "spinner", "label": self.label}


class Skeleton(Component):
    """Placeholder loading lines."""

    def __init__(self, rows: int = 1, height: float = 14.0) -> None:
        self.rows = max(1, int(rows))
        self.height = float(height)

    def to_node(self) -> dict:
        bar = "▬" * 16
        return {
            "type": "column",
            "children": [
                {"type": "text", "text": bar}
                for _ in range(self.rows)
            ],
            "gap": 6.0,
        }


class Pending(Component):
    """Renders ``placeholder`` while ``active`` is true, otherwise ``child``."""

    def __init__(
        self,
        active: bool,
        child: Component,
        placeholder: Component | None = None,
    ) -> None:
        if not isinstance(child, Component):
            raise TypeError(
                f"Pending child must subclass Component, got {type(child).__name__}"
            )
        if placeholder is not None and not isinstance(placeholder, Component):
            raise TypeError(
                f"Pending placeholder must subclass Component, got {type(placeholder).__name__}"
            )
        self.active = bool(active)
        self.child = child
        self.placeholder = placeholder if placeholder is not None else Skeleton(rows=3)

    def to_node(self) -> dict | None:
        target = self.placeholder if self.active else self.child
        return target.to_node()


# ── Data Display & Overlays ───────────────────────────────────────────────────


class KeyValue(Component):
    """Two-column key/value metadata list."""

    def __init__(self, rows: list[tuple[str, str]] | None = None) -> None:
        self.rows = list(rows) if rows is not None else []

    def to_node(self) -> dict:
        children = [
            {
                "type": "row",
                "children": [
                    {"type": "text", "text": f"{k}:", "bold": True},
                    {"type": "text", "text": str(v)},
                ],
                "gap": 8.0,
            }
            for k, v in self.rows
        ]
        return {"type": "column", "children": children, "gap": 4.0}


class CodeBlock(Component):
    """Monospace code block with optional language badge."""

    def __init__(self, code: str = "", language: str = "") -> None:
        self.code = code
        self.language = language

    def to_node(self) -> dict:
        children: list[dict] = []
        if self.language:
            children.append({
                "type": "badge",
                "label": self.language,
                "color": "neutral",
            })
        children.append({
            "type": "text",
            "text": self.code,
            "monospace": True,
        })
        return {"type": "column", "children": children, "gap": 4.0}


class Avatar(Component):
    """Initials badge derived from a display name."""

    def __init__(self, label: str = "", size: float = 28.0) -> None:
        self.label = label
        self.size = float(size)

    def to_node(self) -> dict:
        parts = [p for p in self.label.strip().split() if p]
        initials = "".join(p[0].upper() for p in parts[:2]) or "?"
        return {"type": "badge", "label": initials, "color": "accent"}


class Icon(Component):
    """Named icon glyph."""

    def __init__(self, name: str = "", size: float = 16.0, tone: str = "body") -> None:
        self.name = name
        self.size = float(size)
        self.tone = tone

    def to_node(self) -> dict:
        return {"type": "text", "text": f"[{self.name}]", "size": self.size}


class Tooltip(Component):
    """Inline tooltip hint displayed alongside a child component."""

    def __init__(self, text: str, child: Component) -> None:
        self.text = text
        self.child = child

    def to_node(self) -> dict:
        child_node = self.child.to_node() if hasattr(self.child, "to_node") else self.child
        return {
            "type": "row",
            "children": [
                child_node,
                {"type": "text", "text": f"({self.text})", "size": float(TEXT_CAPTION)},
            ],
            "gap": 6.0,
        }


class Accordion(Component):
    """Collapsible section header + body."""

    def __init__(
        self,
        node_id: str,
        title: str,
        child: Component,
        open: bool = False,
    ) -> None:
        self.node_id = node_id
        self.title = title
        self.child = child
        self.open = bool(open)

    def to_node(self) -> dict:
        glyph = "▾" if self.open else "▸"
        header = {
            "type": "button",
            "label": f"{glyph} {self.title}",
            "on_click": self.node_id,
            "style": "ghost",
            "disabled": False,
        }
        children: list[Any] = [header]
        if self.open:
            child_node = self.child.to_node() if hasattr(self.child, "to_node") else self.child
            if child_node is not None:
                children.append(child_node)
        return {"type": "column", "children": children, "gap": 4.0}


class EmptyState(Component):
    """Centered empty-state message with optional icon and description."""

    def __init__(
        self,
        title: str,
        description: str = "",
        icon: str = "",
    ) -> None:
        self.title = title
        self.description = description
        self.icon = icon

    def to_node(self) -> dict:
        children: list[dict] = []
        if self.icon:
            children.append({
                "type": "text",
                "text": self.icon,
                "size": float(TEXT_TITLE),
                "color": "accent",
                "align": "center",
            })
        else:
            children.append({
                "type": "badge",
                "label": "empty",
                "color": "neutral",
            })
        children.append({
            "type": "text",
            "text": self.title,
            "bold": True,
            "size": float(TEXT_HEADING),
            "align": "center",
        })
        if self.description:
            children.append({
                "type": "text",
                "text": self.description,
                "color": "muted",
                "align": "center",
            })
        return {
            "type": "card",
            "padding": float(SPACE_XL),
            "child": {
                "type": "column",
                "children": children,
                "gap": 6.0,
                "align": "center",
            },
        }

    to_dict = to_node


class Modal(Component):
    """Framed modal dialog block."""

    def __init__(
        self,
        node_id: str,
        title: str,
        child: Component,
        open: bool = True,
    ) -> None:
        self.node_id = node_id
        self.title = title
        self.child = child
        self.open = bool(open)

    def to_node(self) -> dict:
        if not self.open:
            return {"type": "column", "children": [], "gap": 0.0}
        child_node = self.child.to_node() if hasattr(self.child, "to_node") else self.child
        return {
            "type": "column",
            "children": [
                {"type": "divider"},
                {
                    "type": "row",
                    "children": [
                        {"type": "text", "text": self.title, "bold": True},
                        {
                            "type": "button",
                            "label": "×",
                            "on_click": self.node_id,
                            "style": "ghost",
                            "disabled": False,
                        },
                    ],
                    "gap": 8.0,
                },
                child_node,
                {"type": "divider"},
            ],
            "gap": 8.0,
        }


# ── 2D Canvas Primitives ──────────────────────────────────────────────────────

CANVAS_WIRE_DECIMALS: int = 2


def _wire_float(v: float) -> float:
    return round(float(v), CANVAS_WIRE_DECIMALS)


def _require_finite(name: str, value: float) -> float:
    f = float(value)
    if not math.isfinite(f):
        raise ValueError(f"{name} must be finite, got {value!r}")
    return f


def _require_non_negative(name: str, value: float) -> float:
    f = _require_finite(name, value)
    if f < 0.0:
        raise ValueError(f"{name} must be >= 0, got {value!r}")
    return f


@dataclass
class CanvasRect:
    x: float
    y: float
    width: float
    height: float
    fill: str
    radius: float = 0.0

    def to_command(self) -> dict:
        return {
            "type": "rect",
            "x": _wire_float(self.x),
            "y": _wire_float(self.y),
            "w": _wire_float(self.width),
            "h": _wire_float(self.height),
            "fill": self.fill,
            "radius": _wire_float(self.radius),
        }


@dataclass
class CanvasCircle:
    x: float
    y: float
    radius: float
    fill: str

    def to_command(self) -> dict:
        return {
            "type": "circle",
            "cx": _wire_float(self.x),
            "cy": _wire_float(self.y),
            "r": _wire_float(self.radius),
            "fill": self.fill,
        }


@dataclass
class CanvasLine:
    x1: float
    y1: float
    x2: float
    y2: float
    color: str = "#cdd6f4"
    width: float = 1.0

    def to_command(self) -> dict:
        return {
            "type": "line",
            "x1": _wire_float(self.x1),
            "y1": _wire_float(self.y1),
            "x2": _wire_float(self.x2),
            "y2": _wire_float(self.y2),
            "color": self.color,
            "width": _wire_float(self.width),
        }


@dataclass
class CanvasText:
    x: float
    y: float
    text: str
    size: float = 14.0
    color: str = "#ffffff"
    bold: bool = False
    align: str = "left_top"
    monospace: bool = False

    def to_command(self) -> dict:
        return {
            "type": "text",
            "x": _wire_float(self.x),
            "y": _wire_float(self.y),
            "text": self.text,
            "size": _wire_float(self.size),
            "color": self.color,
            "bold": self.bold,
            "align": self.align,
            "monospace": self.monospace,
            "max_width": None,
            "elide": True,
            "selectable": False,
        }


class Canvas(Component):
    """2D vector drawing surface rendered natively by the host."""

    def __init__(
        self,
        commands: list[Any] | None = None,
        width: float = 640.0,
        height: float = 360.0,
        grow: bool = True,
        fit: str = "fill",
        key: str = "",
        on_click: str = "",
    ) -> None:
        if callable(commands):
            raise TypeError("Canvas expects typed canvas commands, not a draw callback")
        if fit not in ("fill", "contain"):
            raise ValueError("Canvas fit must be 'fill' or 'contain'")
        self.commands = list(commands or [])
        self.width = float(width)
        self.height = float(height)
        self.grow = bool(grow)
        self.fit = fit
        self.key = key
        self.on_click = on_click

    def to_node(self) -> dict:
        serialized = [
            cmd.to_command() if hasattr(cmd, "to_command") else cmd
            for cmd in self.commands
        ]
        node: dict[str, Any] = {
            "type": "canvas",
            "width": self.width,
            "height": self.height,
            "grow": self.grow,
            "fit": self.fit,
            "commands": serialized,
        }
        if self.on_click:
            node["on_click"] = self.on_click
        return node


__all__ = [
    # Design tokens
    "SPACE_XS", "SPACE_SM", "SPACE_MD", "SPACE_LG", "SPACE_XL",
    "TEXT_HINT", "TEXT_CAPTION", "TEXT_BODY", "TEXT_HEADING",
    "TEXT_TITLE", "TEXT_TITLE_XL",
    "RADIUS_SM", "RADIUS_MD", "RADIUS_LG", "RADIUS_BADGE",
    "BadgeColor", "BADGE_COLORS",
    "TextColor", "TEXT_COLORS",
    "CANVAS_WIRE_DECIMALS",
    # Base & helpers
    "Component", "Keyed",
    "ensure_visible",
    "move_cursor",
    # Layout containers
    "Column", "HStack", "Row", "Card", "Scrollable", "Grid",
    # Chrome & typography
    "AppBar", "Section", "Heading", "Label", "Text", "Spacer", "Divider",
    "Badge", "FooterKeys",
    # Buttons & form controls
    "Button", "Actions", "ActionBar", "TextInput", "TextEdit", "FormField",
    "Checkbox", "Toggle", "Switch", "Radio", "Slider", "Select", "DateTimePicker",
    # Lists, navigation & tables
    "SelectList", "Breadcrumb", "Tabs", "TabBar", "Pagination",
    "DataTable", "Table",
    # Feedback & status
    "Banner", "ProgressBar", "Progress", "Spinner", "Skeleton", "Pending",
    # Data display & overlays
    "KeyValue", "CodeBlock", "Avatar", "Icon", "Tooltip",
    "Accordion", "EmptyState", "Modal",
    # 2D Canvas
    "Canvas", "CanvasRect", "CanvasCircle", "CanvasLine", "CanvasText",
]
