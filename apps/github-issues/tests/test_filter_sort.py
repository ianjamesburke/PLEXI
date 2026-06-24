import importlib.util
import sys
from pathlib import Path
from types import SimpleNamespace


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

    assert order == ["created desc", "created asc", "number desc", "number asc"]


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
    assert chips[0].color == app.COLOR_DANGER


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


def test_headers_are_unauthenticated_without_token(monkeypatch):
    app = _load_app_module()
    monkeypatch.delenv("GH_TOKEN", raising=False)
    monkeypatch.delenv("GITHUB_TOKEN", raising=False)
    monkeypatch.setattr(
        app.subprocess,
        "run",
        lambda *args, **kwargs: SimpleNamespace(returncode=1, stdout=""),
    )

    headers = app._headers()

    assert "Authorization" not in headers


def test_headers_use_github_token_when_available(monkeypatch):
    app = _load_app_module()
    monkeypatch.delenv("GITHUB_TOKEN", raising=False)
    monkeypatch.setenv("GH_TOKEN", "test-token")

    headers = app._headers()

    assert headers["Authorization"] == "Bearer test-token"


def test_headers_fall_back_to_gh_cli_token(monkeypatch):
    app = _load_app_module()
    monkeypatch.delenv("GH_TOKEN", raising=False)
    monkeypatch.delenv("GITHUB_TOKEN", raising=False)

    def fake_run(args, **kwargs):
        assert args == ["gh", "auth", "token"]
        assert kwargs["capture_output"] is True
        assert kwargs["text"] is True
        return SimpleNamespace(returncode=0, stdout="cli-token\n")

    monkeypatch.setattr(app.subprocess, "run", fake_run)

    headers = app._headers()

    assert headers["Authorization"] == "Bearer cli-token"


def test_headers_ignore_failed_gh_cli_token(monkeypatch):
    app = _load_app_module()
    monkeypatch.delenv("GH_TOKEN", raising=False)
    monkeypatch.delenv("GITHUB_TOKEN", raising=False)
    monkeypatch.setattr(
        app.subprocess,
        "run",
        lambda *args, **kwargs: SimpleNamespace(returncode=1, stdout=""),
    )

    headers = app._headers()

    assert "Authorization" not in headers


def test_picker_toggle_and_apply_filters_by_label():
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    data.update({"issues": _issues(), "view": "picker"})

    effects = app._handle_picker_key(data, "space")
    assert data["picker_staged"] == ["bug"]
    assert effects

    app._handle_picker_key(data, "enter")

    assert data["view"] == "list"
    assert data["filter_labels"] == ["bug"]
    assert [issue["number"] for issue in app._visible_issues(data)] == [5, 1]


def test_list_rows_include_alpha_style_issue_metadata():
    app = _load_app_module()
    issue = _many_label_issues()[0]

    description = app._issue_description(issue, {"v1.0"})

    assert "v1.0" in description
    assert "+2" in description


def test_issue_rows_emit_host_list_view_badges_and_colored_chips():
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    data.update({"filter_labels": ["P0"]})

    rows = app._issue_rows(data, [_many_label_issues()[0]])

    assert rows[0]["type"] == "row"
    assert rows[0]["leading"] == {
        "variant": "badge",
        "label": "#10",
        "color": app.COLOR_ACCENT,
    }
    assert rows[0]["chips"][0] == {"label": "P0", "color": app.COLOR_DANGER}
    assert rows[0]["chips"][-1] == {"label": "+2", "color": app.COLOR_MUTED}


def test_slash_toggles_filter_mode_without_clearing_query(monkeypatch):
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    data.update({"repo": "owner/repo", "filter": "bug", "filter_active": False})
    monkeypatch.setattr(app.state, "get", lambda key, default=None: data.get(key, default))

    effects = app.update(app.KeyEvent(key="/", pressed=True))
    next_state = [effect for effect in effects if isinstance(effect, app.SetState)][0].data

    assert next_state["filter_active"] is True
    assert next_state["filter"] == "bug"


def test_filter_submit_returns_to_navigation_mode(monkeypatch):
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    data.update({"repo": "owner/repo", "issues": _issues(), "filter": "bug", "filter_active": True})
    monkeypatch.setattr(app.state, "get", lambda key, default=None: data.get(key, default))

    effects = app.update(app.UiAction(handler_id="issues-filter"))
    next_state = [effect for effect in effects if isinstance(effect, app.SetState)][0].data

    assert next_state["filter_active"] is False
    assert app._visible_issues(next_state)


def test_host_list_select_updates_selected_index():
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    data.update({"repo": "owner/repo", "issues": _issues(), "selected": 0})

    effects = app._handle_list_select(data, app.ListSelect(id="issues", index=2))

    assert data["selected"] == 2
    assert effects


def test_host_list_activate_opens_detail_fetch():
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    data.update({"repo": "owner/repo", "issues": _issues(), "selected": 0})

    effects = app._handle_list_activate(data, app.ListActivate(id="issues", index=1))

    assert data["view"] == "detail"
    assert data["pending"] == "detail:5"
    assert any(isinstance(effect, app.HttpFetch) for effect in effects)


def test_toggle_filter_preserves_selected_issue_when_it_remains_visible():
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    data.update({"repo": "owner/repo", "issues": _issues(), "selected": 1})

    app._toggle_filter_from_selection(data)

    assert data["filter_labels"] == ["bug",]
    assert app._selected_issue(data)["number"] == 5


def test_issue_url_uses_api_html_url_when_present():
    app = _load_app_module()
    data = {"repo": "owner/repo"}
    issue = {"number": 7, "html_url": "https://github.com/owner/repo/issues/7"}

    assert app._issue_url(data, issue) == "https://github.com/owner/repo/issues/7"


def test_issue_url_falls_back_to_repo_and_number():
    app = _load_app_module()

    assert (
        app._issue_url({"repo": "owner/repo"}, {"number": 7})
        == "https://github.com/owner/repo/issues/7"
    )


def test_new_issue_url_points_to_github_repo():
    app = _load_app_module()

    assert app._new_issue_url({"repo": "owner/repo"}) == "https://github.com/owner/repo/issues/new"


def test_o_key_opens_selected_issue_via_host_open_url(monkeypatch):
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    data.update({"repo": "owner/repo", "issues": _issues(), "selected": 0})
    monkeypatch.setattr(app.state, "get", lambda key, default=None: data.get(key, default))

    effects = app.update(app.KeyEvent(key="o", pressed=True))

    open_urls = [effect for effect in effects if isinstance(effect, app.OpenUrl)]
    assert len(open_urls) == 1
    assert open_urls[0].url == "https://github.com/owner/repo/issues/9"


def test_n_key_opens_new_issue_via_host_open_url(monkeypatch):
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    data.update({"repo": "owner/repo", "issues": _issues()})
    monkeypatch.setattr(app.state, "get", lambda key, default=None: data.get(key, default))

    effects = app.update(app.KeyEvent(key="n", pressed=True))

    open_urls = [effect for effect in effects if isinstance(effect, app.OpenUrl)]
    assert len(open_urls) == 1
    assert open_urls[0].url == "https://github.com/owner/repo/issues/new"
