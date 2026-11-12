#!/usr/bin/env python3
"""Print the message type strings the Python SDK runtime puts on the wire.

Parses sdk/python/plexi_sdk/_v3_runtime.py with `ast` (never regex) and prints
JSON: {"emits": [...], "handles": [...]}.

emits   every dict literal with a constant "type" key: a guest-to-host message.
handles every constant compared against `t` in PluginRuntime._handle: a
        host-to-guest message the runtime dispatches on.

Consumed by the contract test in src/host/wasm_python.rs
(sdk_and_host_agree_on_wire_message_kinds). Fails loudly on a shape it does
not understand rather than returning a partial set.
"""
import ast
import json
import pathlib
import sys

RUNTIME = pathlib.Path(__file__).resolve().parent.parent / "sdk/python/plexi_sdk/_v3_runtime.py"


def main() -> int:
    tree = ast.parse(RUNTIME.read_text(), filename=str(RUNTIME))
    emits = set()
    for node in ast.walk(tree):
        if not isinstance(node, ast.Dict):
            continue
        for key, value in zip(node.keys, node.values):
            if isinstance(key, ast.Constant) and key.value == "type":
                if not (isinstance(value, ast.Constant) and isinstance(value.value, str)):
                    sys.exit(f"{RUNTIME}:{node.lineno}: non-constant wire 'type' value")
                emits.add(value.value)

    handles = set()
    handler = next(
        (
            fn
            for cls in tree.body
            if isinstance(cls, ast.ClassDef) and cls.name == "PluginRuntime"
            for fn in cls.body
            if isinstance(fn, ast.FunctionDef) and fn.name == "_handle"
        ),
        None,
    )
    if handler is None:
        sys.exit(f"{RUNTIME}: PluginRuntime._handle not found")
    for node in ast.walk(handler):
        if (
            isinstance(node, ast.Compare)
            and isinstance(node.left, ast.Name)
            and node.left.id == "t"
            and len(node.ops) == 1
            and isinstance(node.ops[0], ast.Eq)
            and isinstance(node.comparators[0], ast.Constant)
            and isinstance(node.comparators[0].value, str)
        ):
            handles.add(node.comparators[0].value)
    if not handles:
        sys.exit(f"{RUNTIME}: no `t == ...` dispatch found in _handle")
    print(json.dumps({"emits": sorted(emits), "handles": sorted(handles)}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
