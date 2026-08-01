"""State scope tests for the v3 runtime (stint 0652).

An app declares its state scopes in the manifest; the host passes them to the
runtime in the init message (``state_scopes`` + per-scope ``states``). The
runtime addresses state by scope name, emits ``save_app_state`` with an
explicit ``scope``, and refuses a scope the app did not declare — never a
silent fallback to another scope's data.
"""

import json
import textwrap
from pathlib import Path

from test_v3_runtime_regression import (
    _collect_until,
    _find_events,
    _render,
    _spawn_v3_app,
)


def _init_scoped(proc, *, scopes=None, states=None, state=None):
    msg = {
        "type": "init",
        "app_id": "scope-test",
        "workspace_root": "/tmp",
        "capabilities": [],
        "feature_flags": [],
        "protocol": "pgap/3",
        "width": 640.0,
        "height": 360.0,
    }
    if scopes is not None:
        msg["state_scopes"] = scopes
    if states is not None:
        msg["states"] = states
    if state is not None:
        msg["state"] = state
    proc.stdin.write(json.dumps(msg) + "\n")
    proc.stdin.flush()
    return _collect_until(proc, "ready")


def _write_app(tmp_path: Path, init_effects: str, body: str = "") -> Path:
    app = tmp_path / "app.py"
    app.write_text(textwrap.dedent(f"""
        from plexi_sdk import state
        from plexi_sdk.effects import *
        from plexi_sdk.events import *
        from plexi_sdk.ui import Text

        {body}

        def init(size, args):
            return [{init_effects}]

        def update(event):
            return []

        def view():
            return Text("scopes")
    """))
    return app


def test_persist_default_scope_emits_first_declared_scope(tmp_path):
    app = _write_app(tmp_path, 'PersistState({"n": 1})')
    proc = _spawn_v3_app(app)
    try:
        events = _init_scoped(proc, scopes=["context", "global"])
        events += _render(proc)
        saves = _find_events(events, "save_app_state")
        assert len(saves) == 1
        assert saves[0]["scope"] == "context", "default scope is the first declared"
        assert saves[0]["payload"] == {"n": 1}
    finally:
        proc.kill()


def test_persist_explicit_scope_targets_only_that_scope(tmp_path):
    app = _write_app(tmp_path, 'PersistState({"here": True}, scope="context")')
    proc = _spawn_v3_app(app)
    try:
        events = _init_scoped(
            proc,
            scopes=["global", "context"],
            states={"global": {"g": 1}, "context": {"c": 1}},
        )
        events += _render(proc)
        saves = _find_events(events, "save_app_state")
        assert len(saves) == 1
        assert saves[0]["scope"] == "context"
        # The persisted snapshot is the context scope's merged map — the
        # global scope's keys must not bleed in.
        assert saves[0]["payload"] == {"c": 1, "here": True}
    finally:
        proc.kill()


def test_scoped_state_reads_resolve_per_scope(tmp_path):
    app = _write_app(
        tmp_path,
        'PersistState({"echo": [state.get("k", scope="global"), state.get("k", scope="context")]})',
    )
    proc = _spawn_v3_app(app)
    try:
        events = _init_scoped(
            proc,
            scopes=["global", "context"],
            states={"global": {"k": "g"}, "context": {"k": "c"}},
        )
        events += _render(proc)
        saves = _find_events(events, "save_app_state")
        assert len(saves) == 1
        assert saves[0]["payload"]["echo"] == ["g", "c"]
    finally:
        proc.kill()


def test_legacy_flat_state_init_lands_in_default_scope(tmp_path):
    app = _write_app(tmp_path, 'PersistState({"bump": state.get("count", 0) + 1})')
    proc = _spawn_v3_app(app)
    try:
        events = _init_scoped(proc, state={"count": 41})
        events += _render(proc)
        saves = _find_events(events, "save_app_state")
        assert len(saves) == 1
        assert saves[0]["scope"] == "global"
        assert saves[0]["payload"]["bump"] == 42
    finally:
        proc.kill()


def test_undeclared_scope_never_persists(tmp_path):
    app = _write_app(tmp_path, 'PersistState({"x": 1}, scope="context")')
    proc = _spawn_v3_app(app)
    try:
        events = _init_scoped(proc, scopes=["global"])
        try:
            events += _render(proc)
        except Exception:
            pass  # the raising update loop may kill the frame — that is fine
        saves = _find_events(events, "save_app_state")
        assert saves == [], "an undeclared scope must not emit a persist"
    finally:
        proc.kill()
