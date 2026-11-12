"""No text-entry constructor argument may be accepted and then dropped (stint 1336).

`TextInput` once took `multiline`, `max_length` and `height`, stored them, and
left them out of `to_node`, so the host never saw them and the input stayed a
single line with no error. An argument the wire cannot carry must not be in
the signature.
"""
import inspect

import pytest

from plexi_sdk.ui import FormField, TextInput


def _changed_value(parameter: inspect.Parameter):
    default = parameter.default
    if isinstance(default, bool):
        return not default
    if isinstance(default, (int, float)):
        return default + 5
    if default is None:
        return 7
    return f"set-{parameter.name}"


@pytest.mark.parametrize(
    ("component", "required"),
    [
        (TextInput, {}),
        (FormField, {"id": "field", "label": "Field"}),
    ],
)
def test_every_constructor_argument_reaches_the_node(component, required):
    baseline = component(**required).to_node()
    for name, parameter in inspect.signature(component.__init__).parameters.items():
        if name == "self":
            continue
        node = component(**{**required, name: _changed_value(parameter)}).to_node()
        assert node != baseline, (
            f"{component.__name__}({name}=...) is accepted but changes nothing in "
            "to_node(), so the host never sees it"
        )


@pytest.mark.parametrize("name", ["multiline", "max_length", "height"])
def test_text_input_rejects_arguments_the_host_cannot_honor(name):
    with pytest.raises(TypeError, match=name):
        TextInput("note", **{name: 1})
