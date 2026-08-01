"""Form primitives every app needs: focused inputs, labeled fields, action rows."""

from __future__ import annotations

import json

import pytest

from plexi_sdk import tools
from plexi_sdk.effects import ExposeTools, PersistState, ToolResult
from plexi_sdk.events import KeyEvent, ToolCall
from plexi_sdk.ui import Actions, Button, FormField, TextInput


def test_text_input_declares_no_focus_by_default() -> None:
    assert TextInput("q").to_node()["autofocus"] is False


def test_text_input_can_declare_itself_the_default_focus() -> None:
    assert TextInput("q", autofocus=True).to_node()["autofocus"] is True


def test_form_field_composes_a_labeled_input_in_the_declarative_tree() -> None:
    node = FormField("draft", "Item", placeholder="What needs doing?",
                     value="milk", autofocus=True).to_node()
    assert node["type"] == "column"
    label, field = node["children"]
    assert label["type"] == "text" and label["text"] == "Item"
    assert field["type"] == "TextInput"
    assert field["value"] == "milk"
    assert field["placeholder"] == "What needs doing?"
    assert field["autofocus"] is True
    # Handler ids default to the field id, matching TextInput.
    assert field["on_change"] == "draft" and field["on_submit"] == "draft"


def test_form_field_marks_required_fields_in_its_label() -> None:
    node = FormField("draft", "Item", required=True).to_node()
    assert node["children"][0]["text"] == "Item *"


def test_form_field_routes_handlers_when_asked() -> None:
    field = FormField("draft", "Item", on_change="c", on_submit="s").to_node()["children"][1]
    assert field["on_change"] == "c" and field["on_submit"] == "s"


def test_actions_lays_buttons_out_in_one_row() -> None:
    node = Actions([Button("Add", "add", style="primary"),
                    Button("Cancel", "cancel", style="ghost")]).to_node()
    assert node["type"] == "row"
    assert [child["label"] for child in node["children"]] == ["Add", "Cancel"]
    assert [child["on_click"] for child in node["children"]] == ["add", "cancel"]
    assert node["gap"] > 0.0


def test_actions_rejects_non_buttons() -> None:
    with pytest.raises(TypeError):
        Actions([TextInput("q")])  # type: ignore[list-item]


class _ToolFixture:
    """Registers tools against a clean registry and restores it afterwards."""

    def __enter__(self) -> "_ToolFixture":
        self._saved = dict(tools._REGISTRY)
        tools._reset_for_tests()
        return self

    def __exit__(self, *exc: object) -> None:
        tools._reset_for_tests()
        tools._REGISTRY.update(self._saved)


def _call(name: str, **arguments) -> ToolCall:
    return ToolCall("c1", name, json.dumps(arguments), "assistant")


def _dispatch(name: str, **arguments) -> list:
    effects = tools.dispatch(_call(name, **arguments))
    assert effects is not None
    return effects


def test_tool_decorator_builds_schemas_and_exposes_declarations() -> None:
    with _ToolFixture():
        @tools.tool("app.echo", "Echo a string.", {"text": str}, {"text": str},
                    read_only=True)
        def _echo(text: str) -> dict:
            return {"text": text}

        exposed = tools.expose()
        assert isinstance(exposed, ExposeTools)
        (decl,) = exposed.tools
        assert decl.name == "app.echo" and decl.read_only is True
        assert decl.input_schema == {
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
        }
        assert decl.output_schema["properties"] == {"text": {"type": "string"}}


def test_tool_dispatch_returns_result_and_the_tools_own_effects() -> None:
    with _ToolFixture():
        @tools.tool("app.save", "Save a value.", {"value": int})
        def _save(value: int) -> tools.Reply:
            return tools.Reply({"saved": value}, [PersistState({"value": value})])

        result, persisted = _dispatch("app.save", value=7)
        assert isinstance(result, ToolResult)
        assert json.loads(result.output_json or "") == {"saved": 7}
        assert isinstance(persisted, PersistState) and persisted.data == {"value": 7}


def test_dispatch_ignores_events_that_are_not_tool_calls() -> None:
    with _ToolFixture():
        assert tools.dispatch(KeyEvent("a")) is None


def test_unknown_tool_and_raising_tool_report_errors_to_the_assistant() -> None:
    with _ToolFixture():
        @tools.tool("app.boom", "Always fails.")
        def _boom() -> dict:
            raise ValueError("no")

        (unknown,) = _dispatch("app.missing")
        assert unknown.output_json is None
        assert "unknown tool 'app.missing'" in (unknown.error or "")

        (failed,) = _dispatch("app.boom")
        assert failed.output_json is None
        assert failed.error == "ValueError: no"


def test_declaring_an_unsupported_parameter_type_fails_at_declaration() -> None:
    with _ToolFixture():
        with pytest.raises(TypeError):
            @tools.tool("app.bad", "Bad.", {"when": complex})
            def _bad(when: complex) -> dict:
                return {}


def test_duplicate_tool_names_are_rejected() -> None:
    with _ToolFixture():
        @tools.tool("app.one", "One.")
        def _one() -> dict:
            return {}

        with pytest.raises(ValueError):
            @tools.tool("app.one", "Again.")
            def _again() -> dict:
                return {}
