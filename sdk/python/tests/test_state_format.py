"""Blessed markdown checklist codec (stint 0644).

Tolerant reader, strict canonical writer — human/agent edits parse, app
writes converge to one shape.
"""

from plexi_sdk.state_format import ChecklistItem, parse_checklist, render_checklist


def test_round_trip_is_stable():
    items = [
        ChecklistItem(text="buy milk", done=False),
        ChecklistItem(text="ship stint 0644", done=True),
    ]
    rendered = render_checklist(items)
    assert parse_checklist(rendered) == items
    # Canonical output is a fixed point: render(parse(render(x))) == render(x).
    assert render_checklist(parse_checklist(rendered)) == rendered


def test_tolerant_reader_accepts_human_variants():
    text = (
        "# My list\n"
        "\n"
        "some prose that is not an item\n"
        "- [ ] plain unchecked\n"
        "- [x] plain checked\n"
        "* [X] star bullet, capital X\n"
        "  -  [ ]   indented and loosely spaced\n"
        "- [] empty brackets count as unchecked\n"
        "-[x] no space after bullet\n"
    )
    items = parse_checklist(text)
    assert items == [
        ChecklistItem(text="plain unchecked", done=False),
        ChecklistItem(text="plain checked", done=True),
        ChecklistItem(text="star bullet, capital X", done=True),
        ChecklistItem(text="indented and loosely spaced", done=False),
        ChecklistItem(text="empty brackets count as unchecked", done=False),
        ChecklistItem(text="no space after bullet", done=True),
    ]


def test_non_matching_lines_are_skipped_not_errors():
    assert parse_checklist("just prose\n\n## heading\n") == []
    assert parse_checklist("") == []


def test_canonical_writer_shape():
    out = render_checklist(
        [ChecklistItem(text="a", done=False), ChecklistItem(text="b", done=True)]
    )
    assert out == "- [ ] a\n- [x] b\n"
    assert out.endswith("\n"), "canonical output carries a trailing newline"
    assert render_checklist([]) == ""
