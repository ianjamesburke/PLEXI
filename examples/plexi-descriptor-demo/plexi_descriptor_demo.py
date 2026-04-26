#!/usr/bin/env python3
# ruff: noqa: D401
"""plexi-descriptor-demo — a tiny CLI that demonstrates `--plexi`.

This is NOT a Plexi app. It's a standalone CLI showing how a command-line
tool opts in to the Plexi auto-UI standard (issue #188) by responding to
`--plexi` with a JSON descriptor matching schemas/plexi-descriptor-schema.json.

Try it:
    python plexi_descriptor_demo.py --plexi              # emits descriptor JSON
    plexi-alpha descriptor probe python plexi_descriptor_demo.py
"""

from __future__ import annotations

import json
import sys

DESCRIPTOR: dict = {
    # Format-version of the descriptor itself. The host parser is at major 0;
    # bump major when the schema breaks.
    "plexi_version": "0.1",
    "name": "parallax",
    "version": "0.1.0",
    "description": "Video agent pipeline CLI (descriptor demo)",
    "icon": "🎬",
    # Render hint when no command is selected. `list` = browse the commands
    # array; `form`/`output`/`tabs`/`stream` are also valid.
    "default_view": "list",
    "commands": [
        {
            "name": "run",
            "description": "Kick off a footage_edit run in cwd",
            "icon": "▶",
            "ui_hint": "form",
            "args": [
                {
                    "name": "brief",
                    "type": "string",
                    "required": True,
                    "description": "What you want the agent to create",
                    "placeholder": "western cowboy scene, 8 seconds",
                },
            ],
            "flags": [
                {"name": "--test-mode", "type": "bool", "default": False},
            ],
            # Capability hint: this command writes to .parallax/. Plexi can
            # surface a trust prompt before the first run.
            "writes": [".parallax/"],
            # Long-running: stdout streams progress events.
            "streaming": True,
        },
        {
            "name": "status",
            "description": "Print manifest stats",
            "ui_hint": "output",
            "args": [],
            # Hint to the consumer that stdout is structured YAML.
            "output_format": "yaml",
        },
        {
            "name": "project",
            "description": "Project management",
            # Subcommand group — `parallax project new <name>`,
            # `parallax project list`. Recursive: project.commands[].commands[]
            # is also legal (e.g. `git remote add`-style multilevel).
            "commands": [
                {
                    "name": "new",
                    "args": [
                        {"name": "name", "type": "string", "required": True},
                    ],
                },
                {"name": "list"},
            ],
        },
    ],
    # Out-of-band state Plexi should watch for changes (e.g. the agent
    # writes manifest.yaml as it runs; the UI re-renders on each update).
    "live_state": {
        "source": "file",
        "path": ".parallax/manifest.yaml",
        "poll_ms": 1000,
        "format": "yaml",
    },
}


def main(argv: list[str]) -> int:
    if "--plexi" in argv:
        json.dump(DESCRIPTOR, sys.stdout, indent=2)
        sys.stdout.write("\n")
        return 0
    sys.stderr.write(
        "plexi-descriptor-demo: a standalone CLI demonstrating the --plexi standard.\n"
        "Run with --plexi to emit the descriptor JSON.\n"
        "See README.md next to this script for context.\n"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
