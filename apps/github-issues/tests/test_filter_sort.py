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


def test_filter_and_sort_defaults_to_newest_created_first():
    app = _load_app_module()

    visible = app._filter_and_sort_issues(_issues(), None, "created_desc")

    assert [issue["number"] for issue in visible] == [9, 5, 1]


def test_filter_and_sort_composes_label_filter_with_number_sort():
    app = _load_app_module()

    visible = app._filter_and_sort_issues(_issues(), "bug", "number_asc")

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
    gh._filter_label = None
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
    gh._filter_label = None
    gh._sort_mode = "created_desc"

    gh._toggle_filter_from_selection()

    assert gh._filter_label == "bug"
    assert [issue["number"] for issue in gh._visible_issues()] == [5, 1]
    assert gh._selected_issue()["number"] == 5

    gh._toggle_filter_from_selection()

    assert gh._filter_label == "P1"
    assert [issue["number"] for issue in gh._visible_issues()] == [5]
    assert gh._selected_issue()["number"] == 5

    gh._toggle_filter_from_selection()

    assert gh._filter_label is None
    assert gh._selected_issue()["number"] == 5
