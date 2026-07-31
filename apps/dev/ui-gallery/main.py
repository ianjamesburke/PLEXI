"""UI Gallery — a compact V3 reference for declarative SDK components."""

from plexi_sdk.effects import SetTitle
from plexi_sdk.ui import AppBar, Column, Divider, FooterKeys, Heading, Label, Spacer


def init(_size, _args):
    return [SetTitle("UI Gallery")]


def update(_event):
    return []


def view():
    return Column([
        AppBar("UI Gallery", subtitle="SDK component reference"),
        Heading("Typography", level=2),
        Heading("Heading level 1", level=1),
        Heading("Heading level 2", level=2),
        Heading("Heading level 3", level=3),
        Label("Label — body tone (default)"),
        Label("Label — caption tone", tone="caption"),
        Label("Label — hint tone", tone="hint"),
        Spacer(size=8.0),
        Divider(),
        Heading("Footer", level=2),
        FooterKeys([("q", "quit"), ("j/k", "scroll")]),
    ], grow=True)
