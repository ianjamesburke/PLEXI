#!/usr/bin/env python3
"""Workspace Config Tester — POC for #308 Phase 1.

Surfaces three things at a glance:
  1. The resolved workspace root path (or "(no workspace)").
  2. The workspace UUID, parsed out of <root>/.plexi/workspace.toml.
  3. Whether <root>/.plexi/config.toml exists, and a few representative keys
     pulled from it so the user can verify project-level overrides land.

The point is to demo global → project config overlay end-to-end: drop a
`[log] level = "debug"` line into <root>/.plexi/config.toml, hit `r` to
reload, and watch the value change. The host's merge logic decides which
value wins; this app just shows the source-of-truth files.

Keys:
  r — Reload from disk
"""
from __future__ import annotations

from pathlib import Path

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    AppBar,
    Card,
    Column,
    Footer,
    KeyRow,
    Label,
    Section,
    Spacer,
)


# Trivial line-oriented TOML reader. Matches the keys we care about
# (`[section]` headers and `key = "..."` / `key = N` forms). Good enough for
# the POC — the host uses `toml::from_str` for real parsing.
def _read_simple_toml(path: Path) -> dict:
    if not path.is_file():
        return {}
    out: dict = {}
    section: str | None = None
    try:
        text = path.read_text()
    except OSError:
        return {}
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            section = line[1:-1].strip()
            out.setdefault(section, {})
            continue
        if "=" not in line:
            continue
        key, _, value = line.partition("=")
        key = key.strip()
        value = value.strip()
        if value.startswith('"') and value.endswith('"'):
            value = value[1:-1]
        target = out.setdefault(section, {}) if section else out
        target[key] = value
    return out


class WorkspaceConfigTesterApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._reload(ctx)
        self.emit.info("Workspace Config Tester started")

    def _reload(self, ctx: RenderContext) -> None:
        root = ctx.workspace_root or ""
        self._workspace_root = root
        self._workspace_id: str = ""
        self._project_config_present: bool = False
        self._project_keys: dict[str, str] = {}

        if not root:
            return

        plexi_dir = Path(root) / ".plexi"
        ws_path = plexi_dir / "workspace.toml"
        cfg_path = plexi_dir / "config.toml"

        ws_data = _read_simple_toml(ws_path)
        # `id` is at the top level (no [section]).
        self._workspace_id = str(ws_data.get("id", "") or "")

        if cfg_path.is_file():
            self._project_config_present = True
            cfg_data = _read_simple_toml(cfg_path)
            # Surface a few keys that are most useful as overlay demos.
            log_section = cfg_data.get("log") or {}
            theme_section = cfg_data.get("theme") or {}
            self._project_keys = {
                "font_size": str(cfg_data.get("font_size", "(unset)") or "(unset)"),
                "log.level": str(log_section.get("level", "(unset)") or "(unset)"),
                "theme.accent": str(theme_section.get("accent", "(unset)") or "(unset)"),
                "theme_preset": str(cfg_data.get("theme_preset", "(unset)") or "(unset)"),
            }

    def on_key(self, ctx: RenderContext, key: str, _mods: dict) -> None:
        if key.lower() == "r":
            self._reload(ctx)
            self.emit.schedule_render(after_ms=20)

    def on_render(self, ctx: RenderContext) -> None:
        root_label = self._workspace_root or "(no workspace — bare plexi launch)"
        uuid_label = self._workspace_id or "(workspace.toml missing or has no id)"

        config_card_rows: list = []
        if not self._workspace_root:
            config_card_rows.append(
                Label("No workspace adopted. Run `plexi-alpha workspace init` in a directory and re-launch.")
            )
        elif not self._project_config_present:
            config_card_rows.append(
                Label("No .plexi/config.toml — host falls back to global config only.")
            )
            config_card_rows.append(
                Label("Drop a config.toml in .plexi/, hit `r`, and the values appear here.")
            )
        else:
            for k, v in self._project_keys.items():
                config_card_rows.append(Label(f"{k} = {v}"))

        ctx.render(Column([
            AppBar(title="Workspace Config Tester"),
            Label("Phase 1 of #308 — workspace + project config overlay."),
            Section("Workspace"),
            Card([
                Label(f"root: {root_label}"),
                Label(f"id:   {uuid_label}"),
            ]),
            Section(".plexi/config.toml (project-level overrides)"),
            Card(config_card_rows),
            Section("Actions"),
            Card([
                KeyRow("r", "Reload from disk"),
            ]),
            Spacer(grow=True),
            Footer("Project values overlay the global config in ~/.plexi-<channel>/config.toml."),
        ]))


WorkspaceConfigTesterApp().run()
