#!/usr/bin/env python3
"""Print the message type strings the Python SDK runtime puts on the wire.

Parses sdk/python/plexi_sdk/_v3_runtime.py with `ast` (never regex) and prints
JSON: {"emits": [...], "handles": [...], "effect_classes": [...],
"effect_arms": [...]}.

emits   every dict literal with a constant "type" key: a guest-to-host message.
handles every constant compared against `t` in PluginRuntime._handle: a
        host-to-guest message the runtime dispatches on.
effect_classes every class defined in plexi_sdk/effects.py.
effect_arms    every `effects.X` named in a non-raising isinstance() arm of
        PluginRuntime._apply_effects: the effects the live runtime executes.
effect_refused same, for arms whose body raises: effects refused loudly.
        Also fails if the chain lacks a raising final `else`.

Consumed by the contract test in src/host/wasm_python.rs
(sdk_and_host_agree_on_wire_message_kinds). Fails loudly on a shape it does
not understand rather than returning a partial set.
"""
import ast
import json
import pathlib
import sys

SDK = pathlib.Path(__file__).resolve().parent.parent / "sdk/python/plexi_sdk"
RUNTIME = SDK / "_v3_runtime.py"
EFFECTS = SDK / "effects.py"


def _ends_in_raising_else(fn: ast.FunctionDef) -> bool:
    """True when the effect if/elif chain ends in an `else` that raises."""
    for node in ast.walk(fn):
        if isinstance(node, ast.For):
            chain = next((n for n in node.body if isinstance(n, ast.If)), None)
            while chain is not None:
                if not chain.orelse:
                    return False
                if len(chain.orelse) == 1 and isinstance(chain.orelse[0], ast.If):
                    chain = chain.orelse[0]
                    continue
                return any(isinstance(stmt, ast.Raise) for stmt in chain.orelse)
    return False


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

    effect_classes = {
        node.name
        for node in ast.parse(EFFECTS.read_text(), filename=str(EFFECTS)).body
        if isinstance(node, ast.ClassDef)
    }
    apply = next(
        (
            fn
            for cls in tree.body
            if isinstance(cls, ast.ClassDef) and cls.name == "PluginRuntime"
            for fn in cls.body
            if isinstance(fn, ast.FunctionDef) and fn.name == "_apply_effects"
        ),
        None,
    )
    if apply is None:
        sys.exit(f"{RUNTIME}: PluginRuntime._apply_effects not found")
    arms, refused = set(), set()
    for node in ast.walk(apply):
        if not (
            isinstance(node, ast.If)
            and isinstance(node.test, ast.Call)
            and isinstance(node.test.func, ast.Name)
            and node.test.func.id == "isinstance"
            and len(node.test.args) == 2
        ):
            continue
        names = {
            ref.attr
            for ref in ast.walk(node.test.args[1])
            if isinstance(ref, ast.Attribute)
            and isinstance(ref.value, ast.Name)
            and ref.value.id == "effects"
        }
        raises = any(isinstance(stmt, ast.Raise) for stmt in node.body)
        (refused if raises else arms).update(names)
    if not _ends_in_raising_else(apply):
        sys.exit(f"{RUNTIME}: _apply_effects has no raising final else for unknown effects")
    print(
        json.dumps(
            {
                "emits": sorted(emits),
                "handles": sorted(handles),
                "effect_classes": sorted(effect_classes),
                "effect_arms": sorted(arms),
                "effect_refused": sorted(refused),
            }
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
