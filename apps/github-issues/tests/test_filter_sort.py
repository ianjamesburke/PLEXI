import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
APP = ROOT / "apps" / "github-issues" / "main.py"


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


def test_next_sort_mode_cycles_documented_order():
    app = _load_app_module()

    mode = "created_desc"
    modes = []
    for _ in range(5):
        modes.append(mode)
        mode = app._next_sort_mode(mode)

    assert modes == [
        "created_desc",
        "created_asc",
        "number_desc",
        "number_asc",
        "created_desc",
    ]


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


def test_normalize_issues_drops_pull_requests():
    app = _load_app_module()
    raw = [
        {
            "number": 1,
            "title": "issue",
            "labels": [],
            "created_at": "2026-01-01T00:00:00Z",
        },
        {"number": 2, "title": "pr", "labels": [], "pull_request": {}},
    ]

    normalized = app._normalize_issues(raw)

    assert [issue["number"] for issue in normalized] == [1]


