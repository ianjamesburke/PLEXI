#!/usr/bin/env python3
"""Component Gallery — living reference for every host-rendered SDK primitive.

Renders the full placeholder vocabulary settled in stint 0328 (buttons, form
controls, data/display primitives) in one scrollable view.

This is a dev POC (``apps/dev/`` — throwaway), not a maintained Core app: it is
the manual smoke surface and the copy-paste reference for app authors.
"""

from __future__ import annotations

from plexi_sdk import log, state
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import UiAction, UiValueChange
from plexi_sdk.ui import (
    Accordion,
    ActionBar,
    AppBar,
    Avatar,
    Badge,
    Banner,
    Breadcrumb,
    Button,
    Card,
    Checkbox,
    CodeBlock,
    Column,
    DateTimePicker,
    Divider,
    EmptyState,
    FooterKeys,
    HStack,
    Icon,
    KeyValue,
    Label,
    Modal,
    Pagination,
    Progress,
    Radio,
    Section,
    Select,
    Scrollable,
    SelectList,
    Skeleton,
    Slider,
    Spinner,
    Switch,
    TabBar,
    Table,
    Text,
    TextEdit,
    Tooltip,
)

# State keys for the interactive controls.
CHECK = "check"
SWITCH = "switch"
RADIO = "radio"
SLIDER = "slider"
TAB = "tab"
PAGE = "page"
ACCORDION = "accordion"
SELECT = "select"
MODAL = "modal"
NOTE = "note"
LIST = "list"


def init(size, args) -> list:
    log.info("Component Gallery initialized")
    return [
        SetTitle("Component Gallery"),
        SetState({
            CHECK: state.get(CHECK, True),
            SWITCH: state.get(SWITCH, False),
            RADIO: state.get(RADIO, 0),
            SLIDER: state.get(SLIDER, 0.4),
            TAB: state.get(TAB, 0),
            PAGE: state.get(PAGE, 0),
            ACCORDION: state.get(ACCORDION, True),
            SELECT: state.get(SELECT, 0),
            MODAL: state.get(MODAL, False),
            NOTE: state.get(NOTE, ""),
            LIST: state.get(LIST, 0),
        }),
    ]


def _set(key, value) -> list:
    return [SetState({key: value})]


# Scroll container must be a stable instance across renders (offset persists on
# it), so it lives at module level and only its child is rebuilt in view().
_SCROLL = Scrollable(child=Column(children=[]))


def update(event) -> list:
    if isinstance(event, UiValueChange):
        h, v = event.handler_id, event.value
        if h == "gallery-check":
            return _set(CHECK, bool(v))
        if h == "gallery-switch":
            return _set(SWITCH, bool(v))
        if h == "gallery-radio":
            return _set(RADIO, int(v))
        if h == "gallery-slider":
            return _set(SLIDER, float(v))
        if h == "gallery-tabs":
            return _set(TAB, int(v))
        if h == "gallery-page":
            return _set(PAGE, int(v))
        if h == "gallery-note":
            return _set(NOTE, str(v))
    if isinstance(event, UiAction):
        h = event.handler_id
        if h == "gallery-accordion":
            return _set(ACCORDION, not state.get(ACCORDION, True))
        if h == "gallery-modal-open":
            return _set(MODAL, True)
        if h in ("gallery-modal", "gallery-modal-ok"):
            return _set(MODAL, False)
        if h == "gallery-select":
            return _set(SELECT, (state.get(SELECT, 0) + 1) % 3)
    return []


def view():
    children = [
        Label("Every host-rendered SDK primitive (stint 0328). "
              "Scroll with j/k.", tone="caption"),

        Section("Buttons"),
        HStack(children=[
            Button("Primary", "gallery-noop", style="primary"),
            Button("Secondary", "gallery-noop", style="secondary"),
            Button("Ghost", "gallery-noop", style="ghost"),
            Button("Danger", "gallery-noop", style="danger"),
            Button("Disabled", "gallery-noop", disabled=True),
        ]),

        Section("Form controls"),
        Checkbox("gallery-check", label="I agree", checked=state.get(CHECK, True)),
        Switch("gallery-switch", label="Notifications", on=state.get(SWITCH, False)),
        Radio("gallery-radio", options=["Low", "Medium", "High"],
              selected=state.get(RADIO, 0)),
        Slider("gallery-slider", value=state.get(SLIDER, 0.4)),
        Select("gallery-select", options=["Red", "Green", "Blue"],
               selected=state.get(SELECT, 0), placeholder="Pick a colour"),
        DateTimePicker("gallery-date", value="", mode="datetime"),
        TextEdit("gallery-note", value=state.get(NOTE, ""),
                 placeholder="Type a note…"),
        SelectList(items=[
            {"name": "First item", "description": "keyboard-navigable list"},
            {"name": "Second item"},
            {"name": "Third item"},
        ], selected_idx=state.get(LIST, 0)),

        Section("Navigation"),
        Breadcrumb(items=["Home", "Apps", "Gallery"]),
        TabBar("gallery-tabs", tabs=["Overview", "Details", "Logs"],
               active=state.get(TAB, 0)),
        Pagination("gallery-page", page=state.get(PAGE, 0), total=5),

        Section("Feedback"),
        Banner("Saved successfully.", tone="success", title="Success"),
        Banner("Something needs attention.", tone="warning"),
        Progress(value=state.get(SLIDER, 0.4), label="Upload"),
        Progress(indeterminate=True, label="Syncing"),
        Spinner(label="Loading…"),
        Skeleton(rows=3),

        Section("Data display"),
        KeyValue(rows=[("Environment", "production"),
                       ("Region", "us-east-1"),
                       ("Version", "0.1.16")]),
        Table(columns=["Name", "Role", "Status"],
              rows=[["Ada", "Owner", "active"],
                    ["Lin", "Editor", "invited"],
                    ["Sam", "Viewer", "active"]]),
        CodeBlock(code="def view():\n    return Column([...])", language="python"),

        Section("Media & misc"),
        Card(children=[
            Avatar(label="Ian Burke"),
            Label("Avatar renders initials"),
        ]),
        Icon(name="gear"),
        Tooltip(text="This is a tooltip", child=Badge("Hover me", color="accent")),
        Accordion("gallery-accordion", title="Advanced options",
                  open=state.get(ACCORDION, True),
                  child=Text("Hidden content revealed when open.")),

        Section("Empty & dialog"),
        EmptyState(title="No results",
                   description="Try adjusting your filters.", icon="∅"),
        Button("Open modal", "gallery-modal-open", style="secondary"),
        Divider(),
    ]

    if state.get(MODAL, False):
        children.append(
            Modal("gallery-modal", title="Confirm action", child=Column(children=[
                Text("Are you sure?"),
                HStack(children=[
                    Button("Cancel", "gallery-modal", style="ghost"),
                    Button("OK", "gallery-modal-ok", style="primary"),
                ]),
            ]))
        )

    _SCROLL.child = Column(children=children)
    return Column(
        [
            AppBar("Component Gallery", "Every host-rendered primitive"),
            _SCROLL,
            ActionBar([
                Button("Open modal", "gallery-modal-open", style="secondary"),
                Button("Cycle select", "gallery-select", style="ghost"),
            ]),
            FooterKeys([
                ("j/k", "scroll"),
                (["space", "enter"], "toggle"),
                ("esc", "close"),
            ]),
        ],
        grow=True,
        padding=12,
    )
