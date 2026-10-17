import importlib.util
import json
from pathlib import Path

from plexi_sdk import tools
from plexi_sdk.effects import ExposeTools, SetState, ToolResult
from plexi_sdk.events import ToolCall
from plexi_sdk.testing import AppHarness


ROOT = Path(__file__).resolve().parents[3]
APP = ROOT / "apps" / "github-issues" / "main.py"


def _load_app_module():
    tools._reset_for_tests()
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
            "state": "open",
            "createdAt": "2026-01-01T00:00:00Z",
            "labels": [{"name": "bug"}],
        },
        {
            "number": 9,
            "title": "new docs",
            "state": "open",
            "createdAt": "2026-03-01T00:00:00Z",
            "labels": [{"name": "docs"}],
        },
        {
            "number": 5,
            "title": "middle bug",
            "state": "open",
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


# ── Assistant tools (issues.list, issues.open) ───────────────────────────────


def test_init_exposes_read_only_issue_tools():
    app = _load_app_module()
    from plexi_sdk import _v3_state
    from plexi_sdk._v3_state import StateSnapshot

    _v3_state._state = StateSnapshot(dict(app.DEFAULT_STATE), {})
    effects = app.init((800, 600), [])
    exposed = next(e for e in effects if isinstance(e, ExposeTools))
    by_name = {t.name: t for t in exposed.tools}
    assert set(by_name) == {"issues.list", "issues.open"}
    assert all(t.read_only for t in exposed.tools)


def test_tools_dispatch_issues_list_and_open():
    app = _load_app_module()
    from plexi_sdk import _v3_state
    from plexi_sdk._v3_state import StateSnapshot

    seeded = dict(app.DEFAULT_STATE, repo="plexiapp/plexi", issues=_issues())
    _v3_state._state = StateSnapshot(seeded, {})

    # 1. issues.list with no filters returns all issues sorted newest first.
    effects = tools.dispatch(ToolCall("c1", "issues.list", "{}", "assistant"))
    assert effects is not None
    res = next(e for e in effects if isinstance(e, ToolResult))
    assert res.error is None
    payload = json.loads(res.output_json)
    assert [i["number"] for i in payload["issues"]] == [9, 5, 1]

    # 2. issues.list filtered by state, query, and limit.
    effects = tools.dispatch(
        ToolCall(
            "c2",
            "issues.list",
            json.dumps({"state": "open", "query": "bug", "limit": 1}),
            "assistant",
        )
    )
    assert effects is not None
    res = next(e for e in effects if isinstance(e, ToolResult))
    assert res.error is None
    payload = json.loads(res.output_json)
    assert [i["number"] for i in payload["issues"]] == [5]

    # 3. issues.open focuses the matching row in the visible list.
    effects = app.update(
        ToolCall("c3", "issues.open", json.dumps({"number": 1}), "assistant")
    )
    res = next(e for e in effects if isinstance(e, ToolResult))
    assert res.error is None
    opened = json.loads(res.output_json)
    assert opened["number"] == 1
    assert opened["selected"] == 2  # [9, 5, 1] -> index 2
    new_state = next(e for e in effects if isinstance(e, SetState)).data
    assert new_state["selected"] == 2

    # 4. Unknown issue number reports a structured ToolResult error.
    missing = tools.dispatch(
        ToolCall("c4", "issues.open", json.dumps({"number": 999}), "assistant")
    )
    assert missing is not None
    assert missing[0].error is not None


def test_app_harness_exercises_github_issues_tools_end_to_end():
    with AppHarness(str(APP), width=800, height=600) as h:
        h.run(1)
        exposed = [c for c in h._events_seen if c.get("type") == "expose_tools"]
        assert exposed, "init must emit expose_tools"
        names = {t["name"] for t in exposed[-1]["tools"]}
        assert names == {"issues.list", "issues.open"}
        assert all(t["read_only"] for t in exposed[-1]["tools"])

        h._send(
            {
                "type": "inject_state",
                "payload": {"repo": "plexiapp/plexi", "issues": _issues()},
            }
        )
        h._send(
            {
                "type": "tool_call",
                "call_id": "list-1",
                "name": "issues.list",
                "input_json": json.dumps({"query": "bug"}),
                "caller_id": "assistant",
            }
        )
        h.run(1)
        results = [
            c
            for c in h._events_seen
            if c.get("type") == "tool_result" and c.get("call_id") == "list-1"
        ]
        assert results
        out = json.loads(results[-1]["output_json"])
        assert [i["number"] for i in out["issues"]] == [5, 1]

        h._send(
            {
                "type": "tool_call",
                "call_id": "open-1",
                "name": "issues.open",
                "input_json": json.dumps({"number": 5}),
                "caller_id": "assistant",
            }
        )
        h.run(1)
        results = [
            c
            for c in h._events_seen
            if c.get("type") == "tool_result" and c.get("call_id") == "open-1"
        ]
        assert results
        opened = json.loads(results[-1]["output_json"])
        assert opened["number"] == 5
        assert opened["selected"] == 1
        trees = [c for c in h._events_seen if c.get("type") == "component_tree"]
        list_views = [
            n["data"]
            for n in trees[-1]["tree"]["nodes"]
            if n["data"].get("type") == "ListView"
        ]
        assert list_views and list_views[0]["selected"] == 1


def test_no_repo_view_renders_interactive_input_and_detail_view_uses_card():
    app = _load_app_module()
    from plexi_sdk import _v3_state
    from plexi_sdk._v3_state import StateSnapshot
    from plexi_sdk.events import KeyEvent, UiAction, UiValueChange

    # No-repo view includes an interactive TextInput to enter owner/repo.
    _v3_state._state = StateSnapshot(dict(app.DEFAULT_STATE), {})
    no_repo_node = app.view().to_node()
    repo_card = no_repo_node["children"][1]
    assert repo_card["type"] == "card"
    assert any(c.get("type") == "TextInput" for c in repo_card["children"])

    # Submitting owner/repo updates repo state and starts loading.
    effects = app.update(UiValueChange("issues-repo-change", "plexiapp/plexi"))
    st = next(e for e in effects if isinstance(e, SetState)).data
    _v3_state._state = StateSnapshot(st, {})
    effects = app.update(UiAction("issues-repo-submit"))
    st = next(e for e in effects if isinstance(e, SetState)).data
    assert st["repo"] == "plexiapp/plexi"
    assert st["loading"] is True

    # Escape releases filter_autofocus, and "/" restores it.
    seeded = dict(app.DEFAULT_STATE, repo="plexiapp/plexi", issues=_issues(), filter_autofocus=True)
    _v3_state._state = StateSnapshot(seeded, {})
    esc_effects = app.update(KeyEvent("escape"))
    st_after_esc = next(e for e in esc_effects if isinstance(e, SetState)).data
    assert st_after_esc["filter_autofocus"] is False
    _v3_state._state = StateSnapshot(st_after_esc, {})
    slash_effects = app.update(KeyEvent("/"))
    st_after_slash = next(e for e in slash_effects if isinstance(e, SetState)).data
    assert st_after_slash["filter_autofocus"] is True

    # Clicking a row in SelectList opens detail view with Card chrome.
    _v3_state._state = StateSnapshot(st_after_slash, {})
    click_effects = app.update(UiAction("issues-select:1"))
    st_detail = next(e for e in click_effects if isinstance(e, SetState)).data
    assert st_detail["view"] == "detail"
    assert st_detail["selected"] == 1
    assert st_detail["detail"]["number"] == 5
    _v3_state._state = StateSnapshot(st_detail, {})
    detail_node = app.view().to_node()
    scroll_node = detail_node["children"][1]
    assert scroll_node["type"] == "scroll"
    assert scroll_node["child"]["type"] == "card"
