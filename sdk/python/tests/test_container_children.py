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


def test_containers_accept_any_component_sequence():
    """Child params are typed ``Sequence[Component]`` (stint 1362), so a tuple
    must work at runtime too, not just pass mypy."""
    from plexi_sdk.ui import ActionBar, Button, Grid, HStack, Section

    pair = (Label(text="a"), Label(text="b"))
    assert len(Column(pair).children) == 2
    assert len(HStack(pair).children) == 2
    assert len(Card(pair).children) == 2
    assert len(Grid(2, pair).children) == 2
    assert len(Section("Title", pair).children) == 2
    assert len(ActionBar((Button("Go", "go"),)).buttons) == 1


def test_section_rejects_string_children():
    from plexi_sdk.ui import Section

    with pytest.raises(TypeError, match="must subclass Component"):
        Section("Title", "not a list")  # type: ignore[arg-type]


def test_public_type_names_are_exported():
    import plexi_sdk.ui as ui

    for name in ("Component", "BadgeColor", "CanvasCommand"):
        assert name in ui.__all__, name
        assert hasattr(ui, name), name
