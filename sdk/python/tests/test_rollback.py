"""Tests for the undo rollback SDK surface (Phase C):
emit.rollback_verify_result wire shape and the App rollback event handlers
(docs/prm/undo-and-app-events.md)."""

import asyncio
from unittest.mock import patch

import pytest

from plexi_sdk import App


def test_rollback_verify_result_emits_wire_shape():
    emitted = []
    with patch("plexi_sdk._emitter._emit", side_effect=lambda d: emitted.append(d)):
        App().emit.rollback_verify_result("ckpt-1", "rev-13")
    assert emitted == [{
        "type": "rollback_verify_result",
        "checkpoint_id": "ckpt-1",
        "current_revision": "rev-13",
    }]


def test_rollback_verify_result_requires_checkpoint_id():
    with pytest.raises(ValueError, match="checkpoint_id"):
        App().emit.rollback_verify_result("  ", "rev-1")


def test_handle_rollback_verify_answers_with_handler_revision():
    class MyApp(App):
        def on_rollback_verify(self, checkpoint_id, resource_id, expected_revision):
            assert checkpoint_id == "ckpt-7"
            assert resource_id == "game-1"
            assert expected_revision == "rev-3"
            return "rev-3"

    emitted = []
    with patch("plexi_sdk._emitter._emit", side_effect=lambda d: emitted.append(d)):
        asyncio.run(MyApp()._handle_rollback_verify({
            "checkpoint_id": "ckpt-7",
            "resource_id": "game-1",
            "expected_revision": "rev-3",
        }))
    assert emitted == [{
        "type": "rollback_verify_result",
        "checkpoint_id": "ckpt-7",
        "current_revision": "rev-3",
    }]


def test_handle_rollback_verify_default_reports_empty_revision():
    # The base on_rollback_verify returns None — the SDK must still answer,
    # with an empty revision (never matches, rollback safely blocked).
    emitted = []
    with patch("plexi_sdk._emitter._emit", side_effect=lambda d: emitted.append(d)):
        asyncio.run(App()._handle_rollback_verify({
            "checkpoint_id": "ckpt-9",
            "resource_id": "game-1",
            "expected_revision": "rev-3",
        }))
    results = [d for d in emitted if d.get("type") == "rollback_verify_result"]
    assert results == [{
        "type": "rollback_verify_result",
        "checkpoint_id": "ckpt-9",
        "current_revision": "",
    }]


def test_handle_rollback_verify_handler_exception_blocks_safely():
    class BrokenApp(App):
        def on_rollback_verify(self, *_args):
            raise RuntimeError("boom")

    emitted = []
    with patch("plexi_sdk._emitter._emit", side_effect=lambda d: emitted.append(d)):
        asyncio.run(BrokenApp()._handle_rollback_verify({
            "checkpoint_id": "ckpt-2",
            "resource_id": "game-1",
            "expected_revision": "rev-3",
        }))
    results = [d for d in emitted if d.get("type") == "rollback_verify_result"]
    assert len(results) == 1
    assert results[0]["current_revision"] == ""


def test_on_rollback_apply_default_is_noop():
    assert App().on_rollback_apply("ckpt-1", "game-1", "move-3") is None
