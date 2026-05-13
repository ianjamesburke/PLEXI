"""Tests that Column, Card, and Scrollable reject non-Component children at construction."""

import pytest
from plexi_sdk.ui import Column, Card, Scrollable, Label, Component


class _FakeWidget:
    """Ad-hoc class with measure/render but NOT a Component subclass."""
    def measure(self, avail_w: float) -> float:
        return 10.0

    def render(self, ctx, x, y, w, h):
        pass


def test_column_rejects_non_component_child():
    with pytest.raises(TypeError, match="must subclass Component"):
        Column(children=[Label(text="ok"), _FakeWidget()])


def test_column_accepts_component_children():
    col = Column(children=[Label(text="a"), Label(text="b")])
    assert len(col.children) == 2


def test_card_rejects_non_component_child():
    with pytest.raises(TypeError, match="must subclass Component"):
        Card(children=[_FakeWidget()])


def test_card_accepts_component_children():
    card = Card(children=[Label(text="a")])
    assert len(card.children) == 1


def test_scrollable_rejects_non_component_child():
    with pytest.raises(TypeError, match="must subclass Component"):
        Scrollable(child=_FakeWidget())


def test_scrollable_accepts_component_child():
    s = Scrollable(child=Label(text="ok"))
    assert isinstance(s.child, Component)


def test_error_message_names_bad_class():
    with pytest.raises(TypeError, match="_FakeWidget"):
        Column(children=[_FakeWidget()])
