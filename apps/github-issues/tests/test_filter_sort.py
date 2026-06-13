import importlib.util
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
SDK = ROOT / "sdk" / "python"
APP = ROOT / "apps" / "github-issues" / "main.py"

sys.path.insert(0, str(SDK))


def _load_app_module():
    spec = importlib.util.spec_from_file_location("github_issues_app", APP)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _issues():
    return [
        {
            "number": 1,
            "title": "old bug",
            "createdAt": "2026-01-01T00:00:00Z",
            "labels": [{"name": "bug"}],
        },
        {
            "number": 9,
            "title": "new docs",
            "createdAt": "2026-03-01T00:00:00Z",
            "labels": [{"name": "docs"}],
        },
        {
            "number": 5,
            "title": "middle bug",
            "createdAt": "2026-02-01T00:00:00Z",
            "labels": [{"name": "bug"}, {"name": "P1"}],
        },
    ]


def _many_label_issues():
    return [
        {
            "number": 10,
            "title": "complex issue",
            "createdAt": "2026-04-01T00:00:00Z",
            "labels": [
                {"name": "bug"},
                {"name": "P0"},
                {"name": "area:backend"},
                {"name": "v1.0"},
                {"name": "ready"},
            ],
        },
        {
            "number": 11,
            "title": "simple enhancement",
            "createdAt": "2026-04-02T00:00:00Z",
            "labels": [{"name": "enhancement"}, {"name": "P2"}],
        },
        {
            "number": 12,
            "title": "no labels",
            "createdAt": "2026-04-03T00:00:00Z",
            "labels": [],
        },
    ]


def _make_app(app_module, issues=None):
    """Create a GhIssues instance with mocked state, ready for interaction."""
    gh = app_module.GhIssues()
    gh._issues = issues if issues is not None else _issues()
    gh._sel = 0
    gh._filter_labels = set()
    gh._sort_mode = "created_desc"
    gh._view = gh.VIEW_LIST
    gh._loading = False
    gh._detail_loading = False
    gh._error = None
    gh._detail = None
    gh._picker_query = ""
    gh._picker_sel = 0
    gh._picker_staged = set()
    return gh


# ── filter + sort (existing) ────────────────────────────────────────────────

def test_filter_and_sort_defaults_to_newest_created_first():
    app = _load_app_module()

    visible = app._filter_and_sort_issues(_issues(), set(), "created_desc")

    assert [issue["number"] for issue in visible] == [9, 5, 1]


def test_filter_and_sort_composes_label_filter_with_number_sort():
    app = _load_app_module()

    visible = app._filter_and_sort_issues(_issues(), {"bug"}, "number_asc")

    assert [issue["number"] for issue in visible] == [1, 5]


def test_sort_cycle_uses_documented_order():
    app = _load_app_module()

    mode = "created_desc"
    order = []
    for _ in range(4):
        order.append(app.SORT_LABELS[mode])
        mode = app._next_sort_mode(mode)

    assert order == ["created ↓", "created ↑", "number ↓", "number ↑"]


def test_issue_list_limit_is_large_enough_for_active_repos():
    app = _load_app_module()

    assert app.ISSUE_LIST_LIMIT == "500"


def test_app_sort_preserves_selected_index():
    app = _load_app_module()
    gh = _make_app(app)

    gh._cycle_sort()

    assert gh._sort_mode == "created_asc"
    assert gh._sel == 0
    assert gh._selected_issue()["number"] == 1

    gh._cycle_sort()

    assert gh._sort_mode == "number_desc"
    assert gh._sel == 0
    assert gh._selected_issue()["number"] == 9


def test_app_filter_cycles_selected_issue_labels_then_clears():
    app = _load_app_module()
    gh = _make_app(app)
    gh._sel = 1

    gh._toggle_filter_from_selection()

    assert gh._filter_labels == {"bug"}
    assert [issue["number"] for issue in gh._visible_issues()] == [5, 1]
    assert gh._selected_issue()["number"] == 5

    gh._toggle_filter_from_selection()

    assert gh._filter_labels == {"P1"}
    assert [issue["number"] for issue in gh._visible_issues()] == [5]
    assert gh._selected_issue()["number"] == 5

    gh._toggle_filter_from_selection()

    assert gh._filter_labels == set()
    assert gh._selected_issue()["number"] == 5


# ── multi-label AND filter ───────────────────────────────────────────────────

def test_multi_label_and_filter():
    app = _load_app_module()
    issues = _issues()

    visible = app._filter_and_sort_issues(issues, {"bug", "P1"}, "created_desc")

    assert [issue["number"] for issue in visible] == [5]


def test_multi_label_filter_no_match():
    app = _load_app_module()
    issues = _issues()

    visible = app._filter_and_sort_issues(issues, {"bug", "docs"}, "created_desc")

    assert visible == []


def test_empty_filter_returns_all():
    app = _load_app_module()
    issues = _issues()

    visible = app._filter_and_sort_issues(issues, set(), "created_desc")

    assert len(visible) == 3


# ── smart chip selection ─────────────────────────────────────────────────────

def test_chip_selection_prioritizes_active_filter():
    app = _load_app_module()
    issue = _many_label_issues()[0]

    chips = app._select_visible_chips(issue, {"v1.0"})

    assert chips[0].label == "v1.0"


def test_chip_selection_priority_labels_before_rest():
    app = _load_app_module()
    issue = _many_label_issues()[0]

    chips = app._select_visible_chips(issue, set())
    chip_labels = [c.label for c in chips if not c.label.startswith("+")]

    assert "bug" in chip_labels
    assert "P0" in chip_labels


def test_chip_selection_overflow_count():
    app = _load_app_module()
    issue = _many_label_issues()[0]

    chips = app._select_visible_chips(issue, set())

    overflow = [c for c in chips if c.label.startswith("+")]
    assert len(overflow) == 1
    total_labels = len(app._issue_labels(issue))
    visible_count = app.MAX_VISIBLE_CHIPS
    assert overflow[0].label == f"+{total_labels - visible_count}"


def test_chip_selection_no_overflow_when_few_labels():
    app = _load_app_module()
    issue = _many_label_issues()[1]

    chips = app._select_visible_chips(issue, set())

    overflow = [c for c in chips if c.label.startswith("+")]
    assert len(overflow) == 0


def test_chip_selection_no_labels():
    app = _load_app_module()
    issue = _many_label_issues()[2]

    chips = app._select_visible_chips(issue, set())

    assert chips == []


# ── unique label collection ──────────────────────────────────────────────────

def test_collect_unique_labels_sorted():
    app = _load_app_module()
    issues = _issues()

    labels = app._collect_unique_labels(issues)

    assert labels == sorted(labels, key=str.lower)
    assert len(labels) == len(set(labels))
    assert "bug" in labels
    assert "docs" in labels
    assert "P1" in labels


def test_collect_unique_labels_dedupes():
    app = _load_app_module()
    issues = _issues()

    labels = app._collect_unique_labels(issues)

    assert labels.count("bug") == 1


# ── fuzzy match ──────────────────────────────────────────────────────────────

def test_fuzzy_match_case_insensitive():
    app = _load_app_module()

    assert app._fuzzy_match("bug", "Bug Fix")
    assert app._fuzzy_match("BUG", "bug")
    assert not app._fuzzy_match("xyz", "bug")


def test_fuzzy_match_substring():
    app = _load_app_module()

    assert app._fuzzy_match("enhance", "enhancement")
    assert app._fuzzy_match("p1", "P1")
    assert not app._fuzzy_match("p1", "P2")


# ── picker state ─────────────────────────────────────────────────────────────

def test_picker_opens_with_current_filters():
    app = _load_app_module()
    gh = _make_app(app)
    gh._filter_labels = {"bug"}

    gh._open_picker()

    assert gh._view == gh.VIEW_PICKER
    assert gh._picker_staged == {"bug"}
    assert gh._picker_query == ""
    assert gh._picker_sel == 0


def test_picker_apply_sets_filters():
    app = _load_app_module()
    gh = _make_app(app)
    gh._view = gh.VIEW_PICKER
    gh._picker_staged = {"bug", "P1"}

    gh._apply_picker()

    assert gh._view == gh.VIEW_LIST
    assert gh._filter_labels == {"bug", "P1"}


def test_picker_toggle_adds_and_removes():
    """Space toggles the selected label in the picker (SDK normalizes ' ' to 'space')."""
    app = _load_app_module()
    gh = _make_app(app)
    gh._view = gh.VIEW_PICKER

    filtered = gh._picker_filtered_labels()
    first_label = filtered[0]

    gh._handle_picker_key("space")
    assert first_label in gh._picker_staged

    gh._handle_picker_key("space")
    assert first_label not in gh._picker_staged


def test_picker_text_filter_narrows_labels():
    app = _load_app_module()
    gh = _make_app(app)

    all_labels = gh._picker_filtered_labels()

    gh._picker_query = "bug"
    filtered = gh._picker_filtered_labels()

    assert len(filtered) < len(all_labels)
    assert all("bug" in l.lower() for l in filtered)


def test_picker_backspace_removes_char():
    """SDK normalizes 'Backspace' to 'backspace'."""
    app = _load_app_module()
    gh = _make_app(app)
    gh._picker_query = "bug"

    gh._handle_picker_key("backspace")

    assert gh._picker_query == "bu"


def test_picker_typing_appends_chars():
    app = _load_app_module()
    gh = _make_app(app)

    gh._handle_picker_key("b")
    gh._handle_picker_key("u")

    assert gh._picker_query == "bu"


# ── subtitle with multi-label ────────────────────────────────────────────────

def test_subtitle_shows_multi_label_joined():
    app = _load_app_module()
    gh = _make_app(app)
    gh._filter_labels = {"P1", "bug"}

    subtitle = gh._list_subtitle()

    assert "label:P1+bug" in subtitle


def test_subtitle_no_label_when_empty_filter():
    app = _load_app_module()
    gh = _make_app(app)

    subtitle = gh._list_subtitle()

    assert "label:" not in subtitle


# ── end-to-end interaction flows ─────────────────────────────────────────────
# These simulate the exact event sequence the host sends to the SDK.
# Key names use SDK-normalized form (what on_key actually receives).

def test_e2e_open_picker_toggle_apply():
    """Full flow: l → navigate → space toggle → enter apply."""
    app = _load_app_module()
    gh = _make_app(app)
    assert gh._view == gh.VIEW_LIST

    # User presses 'l' → opens picker
    gh._open_picker()
    assert gh._view == gh.VIEW_PICKER
    assert gh._picker_staged == set()

    # Picker shows all labels sorted. Get the list.
    labels = gh._picker_filtered_labels()
    assert len(labels) == 3  # bug, docs, P1
    assert labels == sorted(labels, key=str.lower)

    # Host sends list_select for j/k navigation
    gh.on_list_select("label-picker", 0)
    assert gh._picker_sel == 0

    # User presses space to toggle first label
    gh._handle_picker_key("space")
    assert labels[0] in gh._picker_staged
    assert len(gh._picker_staged) == 1

    # Navigate to second label
    gh.on_list_select("label-picker", 1)
    assert gh._picker_sel == 1

    # Toggle second label
    gh._handle_picker_key("space")
    assert labels[1] in gh._picker_staged
    assert len(gh._picker_staged) == 2

    # User presses enter → host sends list_activate
    gh.on_list_activate("label-picker", 1)
    assert gh._view == gh.VIEW_LIST
    assert gh._filter_labels == {labels[0], labels[1]}

    # Visible issues filtered by AND of both labels
    visible = gh._visible_issues()
    for issue in visible:
        issue_labels = set(app._issue_labels(issue))
        assert gh._filter_labels <= issue_labels


def test_e2e_picker_cancel_preserves_filter():
    """Escape from picker does not change existing filter."""
    app = _load_app_module()
    gh = _make_app(app)
    gh._filter_labels = {"bug"}

    gh._open_picker()
    assert gh._picker_staged == {"bug"}

    # Toggle off the existing filter in picker
    labels = gh._picker_filtered_labels()
    bug_idx = labels.index("bug")
    gh.on_list_select("label-picker", bug_idx)
    gh._handle_picker_key("space")
    assert "bug" not in gh._picker_staged

    # Escape cancels without applying
    gh.on_escape()
    assert gh._view == gh.VIEW_LIST
    assert gh._filter_labels == {"bug"}


def test_e2e_picker_type_to_filter_then_toggle():
    """Type characters to narrow the label list, then toggle."""
    app = _load_app_module()
    gh = _make_app(app)
    gh._open_picker()

    # Type "bu" to filter
    gh._handle_picker_key("b")
    gh._handle_picker_key("u")
    assert gh._picker_query == "bu"

    filtered = gh._picker_filtered_labels()
    assert len(filtered) == 1
    assert filtered[0] == "bug"

    # Toggle the filtered result
    gh._handle_picker_key("space")
    assert "bug" in gh._picker_staged

    # Apply
    gh.on_list_activate("label-picker", 0)
    assert gh._filter_labels == {"bug"}
    assert gh._view == gh.VIEW_LIST


def test_e2e_picker_backspace_widens_filter():
    """Backspace removes last char from query, widening the label list."""
    app = _load_app_module()
    gh = _make_app(app)
    gh._open_picker()

    gh._handle_picker_key("b")
    gh._handle_picker_key("u")
    gh._handle_picker_key("g")
    assert gh._picker_query == "bug"
    assert len(gh._picker_filtered_labels()) == 1

    gh._handle_picker_key("backspace")
    assert gh._picker_query == "bu"
    assert len(gh._picker_filtered_labels()) == 1

    gh._handle_picker_key("backspace")
    assert gh._picker_query == "b"
    assert len(gh._picker_filtered_labels()) == 1  # still just "bug"

    gh._handle_picker_key("backspace")
    assert gh._picker_query == ""
    assert len(gh._picker_filtered_labels()) == 3  # all labels


def test_e2e_f_still_cycles_after_picker():
    """f key still works after using the picker."""
    app = _load_app_module()
    gh = _make_app(app)

    # Use picker to set a multi-label filter
    gh._open_picker()
    gh._handle_picker_key("space")
    gh.on_list_activate("label-picker", 0)
    assert len(gh._filter_labels) == 1

    # Clear filter first so all issues are visible, then select issue #5
    gh._clear_filter()
    # Visible issues (created_desc): [#9, #5, #1]. #5 is at index 1.
    gh._sel = 1
    assert gh._selected_issue()["number"] == 5

    # Press f — should set filter to first label of issue #5 (bug)
    gh._toggle_filter_from_selection()
    assert gh._filter_labels == {"bug"}

    # Press f again — should cycle to next label (P1)
    gh._toggle_filter_from_selection()
    assert gh._filter_labels == {"P1"}


def test_e2e_clear_clears_picker_filters():
    """c key clears multi-label filters set by picker."""
    app = _load_app_module()
    gh = _make_app(app)

    # Apply multi-label filter via picker
    gh._open_picker()
    gh._handle_picker_key("space")
    gh.on_list_select("label-picker", 1)
    gh._handle_picker_key("space")
    gh.on_list_activate("label-picker", 1)
    assert len(gh._filter_labels) == 2

    # Clear
    gh._clear_filter()
    assert gh._filter_labels == set()
    assert len(gh._visible_issues()) == 3


def test_e2e_picker_preserves_selection_after_apply():
    """After applying a filter, the selected issue is clamped to visible range."""
    app = _load_app_module()
    gh = _make_app(app)
    gh._sel = 2  # last issue

    # Filter to only issues with "docs" label (just issue #9)
    gh._open_picker()
    labels = gh._picker_filtered_labels()
    docs_idx = labels.index("docs")
    gh.on_list_select("label-picker", docs_idx)
    gh._handle_picker_key("space")
    gh.on_list_activate("label-picker", docs_idx)

    assert gh._filter_labels == {"docs"}
    visible = gh._visible_issues()
    assert len(visible) == 1
    assert gh._sel <= len(visible) - 1


def test_e2e_list_events_route_to_correct_view():
    """list_select/list_activate route to picker vs issues list by id."""
    app = _load_app_module()
    gh = _make_app(app)

    # In list view, list events go to issues
    gh.on_list_select("issues", 1)
    assert gh._sel == 1
    assert gh._picker_sel == 0

    # In picker view, list events go to picker
    gh._open_picker()
    gh.on_list_select("label-picker", 2)
    assert gh._picker_sel == 2
    assert gh._sel == 1  # unchanged

    # Activate on picker applies, activate on issues opens detail
    gh.on_list_activate("label-picker", 2)
    assert gh._view == gh.VIEW_LIST


def test_e2e_sort_composes_with_multi_label_filter():
    """Sort mode works correctly with multi-label AND filter."""
    app = _load_app_module()
    gh = _make_app(app, issues=_many_label_issues())

    # Filter to issues with both "bug" and "P0" (only issue #10)
    gh._filter_labels = {"bug", "P0"}
    visible = gh._visible_issues()
    assert len(visible) == 1
    assert visible[0]["number"] == 10

    # Changing sort shouldn't break the filter
    gh._cycle_sort()
    visible = gh._visible_issues()
    assert len(visible) == 1
    assert visible[0]["number"] == 10
