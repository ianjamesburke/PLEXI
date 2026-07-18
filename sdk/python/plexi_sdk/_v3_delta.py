"""Diffed component-tree protocol (stint 0438).

The guest emits a full ``component_tree`` only when the tree's shape changes;
otherwise it emits a ``tree_delta`` carrying just the arena slots that changed.
Node identity is the arena index produced by ``_encode_uitree`` — stable
frame-to-frame while the tree keeps the same root, node count, and per-slot
key. This module owns both halves of that contract:

- ``node_patch`` (emit side): shrink one changed slot to the smallest patch.
- ``apply_delta`` / ``TreeReconstructor`` (consume side): rebuild the full
  arena the host will decode, so headless harnesses and tests can assert on a
  reconstructed tree exactly as the live host does.
"""

from __future__ import annotations


class DeltaApplyError(Exception):
    """A ``tree_delta`` could not be applied to the previous tree.

    Raised when a patch references a slot or canvas command index that does not
    exist in the base tree, or when there is no base tree at all. The host
    treats this as a desync and requests a full-tree resync; consumers that
    cannot resync (test harnesses) surface it as a failure rather than paint a
    corrupt tree.
    """


def node_patch(node: dict, old: dict) -> dict:
    """Patch entry for one changed arena slot.

    A canvas node whose only change is same-length command mutations shrinks to
    a ``commands_changed`` entry (games mutate a handful of commands per frame);
    anything else replaces the whole node.
    """
    data = node["data"]
    old_data = old["data"]
    if (
        data.get("type") in ("canvas", "Canvas")
        and data.get("type") == old_data.get("type")
        and len(data.get("commands", ())) == len(old_data.get("commands", ()))
        and {k: v for k, v in data.items() if k != "commands"}
        == {k: v for k, v in old_data.items() if k != "commands"}
    ):
        changed_commands = [
            [index, command]
            for index, (command, old_command) in enumerate(
                zip(data["commands"], old_data["commands"])
            )
            if command != old_command
        ]
        return {
            "id": node["id"],
            "key": node["key"],
            "commands_changed": changed_commands,
        }
    return node


def apply_delta(prev: dict, changed: list) -> dict:
    """Rebuild the full encoded arena from ``prev`` plus a ``changed`` list.

    ``prev`` is the previously emitted encoded tree (``{"root", "nodes"}``).
    Deltas never change the root (a root change forces a full frame), so the
    result carries ``prev``'s root. Raises :class:`DeltaApplyError` on any patch
    that does not fit the base tree — the same fail-loud contract the host's
    Rust decoder enforces.
    """
    nodes = list(prev["nodes"])
    for patch in changed:
        index = patch["id"]
        if not 0 <= index < len(nodes):
            raise DeltaApplyError(
                f"delta patch id {index} out of range (arena has {len(nodes)} nodes)"
            )
        if "commands_changed" in patch:
            base_node = nodes[index]
            data = dict(base_node["data"])
            commands = list(data.get("commands", []))
            for command_index, command in patch["commands_changed"]:
                if not 0 <= command_index < len(commands):
                    raise DeltaApplyError(
                        f"commands_changed index {command_index} out of range "
                        f"(node {index} has {len(commands)} commands)"
                    )
                commands[command_index] = command
            data["commands"] = commands
            nodes[index] = {**base_node, "data": data}
        else:
            nodes[index] = patch
    return {"root": prev["root"], "nodes": nodes}


class TreeReconstructor:
    """Stateful consumer that turns a delta stream back into full trees.

    Feed every protocol event through :meth:`ingest`. ``component_tree`` events
    pass through and become the new base; ``tree_delta`` events are rebuilt into
    an equivalent ``component_tree`` so downstream code only ever sees full
    trees. Every other event passes through untouched.
    """

    def __init__(self) -> None:
        self._last_tree: dict | None = None

    def ingest(self, event: dict) -> dict:
        event_type = event.get("type")
        if event_type == "component_tree":
            self._last_tree = event.get("tree")
            return event
        if event_type == "tree_delta":
            if self._last_tree is None:
                raise DeltaApplyError("tree_delta received before any full tree")
            rebuilt = apply_delta(self._last_tree, event.get("changed", []))
            self._last_tree = rebuilt
            return {
                "type": "component_tree",
                "frame_id": event.get("frame_id"),
                "tree": rebuilt,
            }
        return event
