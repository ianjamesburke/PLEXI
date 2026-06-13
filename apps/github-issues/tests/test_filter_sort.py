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
    gh = app.GhIssues()
    gh._issues = _issues()
    gh._sel = 0
    gh._filter_labels = set()
    gh._sort_mode = "created_desc"

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
    gh = app.GhIssues()
    gh._issues = _issues()
    gh._sel = 1
    gh._filter_labels = set()
    gh._sort_mode = "created_desc"

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
    gh = app.GhIssues()
    gh._issues = _issues()
    gh._filter_labels = {"bug"}
    gh._view = gh.VIEW_LIST

    gh._open_picker()

    assert gh._view == gh.VIEW_PICKER
    assert gh._picker_staged == {"bug"}
    assert gh._picker_query == ""
    assert gh._picker_sel == 0


def test_picker_apply_sets_filters():
    app = _load_app_module()
    gh = app.GhIssues()
    gh._issues = _issues()
    gh._filter_labels = set()
    gh._sort_mode = "created_desc"
    gh._sel = 0
    gh._view = gh.VIEW_PICKER
    gh._picker_staged = {"bug", "P1"}

    gh._apply_picker()

    assert gh._view == gh.VIEW_LIST
    assert gh._filter_labels == {"bug", "P1"}


def test_picker_toggle_adds_and_removes():
    app = _load_app_module()
    gh = app.GhIssues()
    gh._issues = _issues()
    gh._picker_query = ""
    gh._picker_staged = set()
    gh._picker_sel = 0

    filtered = gh._picker_filtered_labels()
    first_label = filtered[0]

    gh._handle_picker_key(" ")
    assert first_label in gh._picker_staged

    gh._handle_picker_key(" ")
    assert first_label not in gh._picker_staged


def test_picker_text_filter_narrows_labels():
    app = _load_app_module()
    gh = app.GhIssues()
    gh._issues = _issues()
    gh._picker_query = ""
    gh._picker_staged = set()

    all_labels = gh._picker_filtered_labels()

    gh._picker_query = "bug"
    filtered = gh._picker_filtered_labels()

    assert len(filtered) < len(all_labels)
    assert all("bug" in l.lower() for l in filtered)


def test_picker_backspace_removes_char():
    app = _load_app_module()
    gh = app.GhIssues()
    gh._issues = _issues()
    gh._picker_query = "bug"
    gh._picker_staged = set()

    gh._handle_picker_key("Backspace")

    assert gh._picker_query == "bu"


def test_picker_typing_appends_chars():
    app = _load_app_module()
    gh = app.GhIssues()
    gh._issues = _issues()
    gh._picker_query = ""
    gh._picker_staged = set()

    gh._handle_picker_key("b")
    gh._handle_picker_key("u")

    assert gh._picker_query == "bu"


# ── subtitle with multi-label ────────────────────────────────────────────────

def test_subtitle_shows_multi_label_joined():
    app = _load_app_module()
    gh = app.GhIssues()
    gh._issues = _issues()
    gh._filter_labels = {"P1", "bug"}
    gh._sort_mode = "created_desc"

    subtitle = gh._list_subtitle()

    assert "label:P1+bug" in subtitle


def test_subtitle_no_label_when_empty_filter():
    app = _load_app_module()
    gh = app.GhIssues()
    gh._issues = _issues()
    gh._filter_labels = set()
    gh._sort_mode = "created_desc"

    subtitle = gh._list_subtitle()

    assert "label:" not in subtitle
