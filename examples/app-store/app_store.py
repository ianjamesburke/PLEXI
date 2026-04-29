#!/usr/bin/env python3
"""App Store — Plexi installer UI (#308 Phase 3).

Three views, switchable with `1` / `2` / `3`:

  1. Browse — fetch a curated registry index over HTTP, list apps with
     install state, capability preview, source URL.
  2. Install by URL — paste a `github:owner/repo[@ref]` or git URL,
     fetch the manifest, show capability preview, confirm, install.
  3. Installed — every app in `~/.plexi-<channel>/apps/`, with
     [Update] / [Uninstall] / Update-all / Export-pack actions.

Network and git both run in the host process: the app shells out to
`plexi-<channel> install/uninstall/update/pack` and to `git ls-remote`
via `subprocess`. We never touch the user's home dir directly except
to read manifests; mutations always go through the host CLI so the
single source of truth (`crate::install`) stays canonical.

Capability approval is mandatory before any install — the user sees
exactly which manifest-declared capabilities they're granting.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import threading
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    AppBar,
    Card,
    Column,
    Footer,
    KeyRow,
    Label,
    Scrollable,
    Section,
    Spacer,
)


# Default registry index. Curated list of installable Plexi apps.
# v3.2: this repo doesn't exist yet; the app falls back to the
# "Install by URL" tab on 404 / network failure with a clear message.
DEFAULT_REGISTRY_URL = (
    "https://raw.githubusercontent.com/ianjamesburke/"
    "plexi-apps-registry/main/index.json"
)

# Human-readable labels for known capability strings. Mirrors the
# spec in #308 Phase 3 — these are the strings declared in
# `manifest.toml::[app.capabilities]::capabilities`.
CAPABILITY_LABELS: dict[str, str] = {
    "network": "Can make network requests",
    "net.http": "Can make HTTP requests",
    "filesystem.read": "Can read files in the workspace",
    "filesystem.write": "Can write files in the workspace",
    "fs.read": "Can read files in the workspace",
    "fs.write": "Can write files in the workspace",
    "secrets": "Can read declared secrets",
    "secrets.get": "Can read declared secrets",
    "terminal": "Can spawn terminals",
    "spawn.app": "Can spawn other apps",
    "audio.record": "Can record audio",
    "audio.playback": "Can play audio",
    "video.playback": "Can play video",
    "pipe.open": "Can open named pipes",
}


def capability_label(name: str) -> str:
    """Render a capability string as a human-readable risk hint.

    Used by every capability-preview surface in the app (browse confirm,
    install-by-url confirm). Unknown capabilities pass through verbatim
    with an `(unknown)` suffix so the user sees the raw string and can
    audit it themselves.
    """
    if not name:
        return "(empty capability)"
    if name in CAPABILITY_LABELS:
        return f"{name}: {CAPABILITY_LABELS[name]}"
    return f"{name} (unknown)"


# ── Trivial line-oriented TOML reader ────────────────────────────────────────
# Same shape as workspace-config-tester's helper. Good enough to read
# `manifest.toml` for the install-by-url manifest preview and the installed
# list. Real parsing (with quoting, arrays, etc.) happens host-side via
# `toml::from_str` — this is purely a UI affordance.
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
        value = value.strip().rstrip(",")
        if value.startswith('"') and value.endswith('"'):
            value = value[1:-1]
        target = out.setdefault(section, {}) if section else out
        target[key] = value
    return out


def parse_capabilities_from_manifest_text(text: str) -> list[str]:
    """Pull the `capabilities = [...]` list out of a manifest.toml text.

    The simple TOML reader above doesn't handle inline arrays. This is a
    line-oriented best-effort extractor for the install-by-URL preview —
    same trade-off the host already accepts in workspace-config-tester.
    """
    in_caps_section = False
    for raw in text.splitlines():
        line = raw.strip()
        if line.startswith("[") and line.endswith("]"):
            in_caps_section = line == "[app.capabilities]"
            continue
        if not in_caps_section:
            continue
        if line.startswith("capabilities"):
            _, _, value = line.partition("=")
            value = value.strip()
            if value.startswith("[") and value.endswith("]"):
                inner = value[1:-1]
                items: list[str] = []
                for tok in inner.split(","):
                    tok = tok.strip().strip('"').strip("'")
                    if tok:
                        items.append(tok)
                return items
    return []


# ── Installed-apps scan ──────────────────────────────────────────────────────
@dataclass
class InstalledRow:
    id: str
    version: str
    source: str  # "git" | "local" | "(unknown)"
    capabilities: list[str] = field(default_factory=list)
    path: str = ""


def _channel_apps_dir() -> Optional[Path]:
    """Walk up from this file's location to find the `apps` dir of the
    current `~/.plexi-<channel>/` install. Returns None when the app is
    being run from somewhere other than a channel install (e.g. tests)."""
    here = Path(__file__).resolve()
    apps_dir = here.parent.parent
    if apps_dir.name == "apps" and apps_dir.parent.name.startswith(".plexi"):
        return apps_dir
    return None


def scan_installed_apps(apps_dir: Optional[Path] = None) -> list[InstalledRow]:
    """List every installed app under the channel apps dir. Pure
    function — accepts an explicit `apps_dir` for tests, otherwise
    derives it from `__file__`'s location."""
    if apps_dir is None:
        apps_dir = _channel_apps_dir()
    if apps_dir is None or not apps_dir.is_dir():
        return []
    rows: list[InstalledRow] = []
    try:
        entries = sorted(apps_dir.iterdir())
    except OSError:
        return []
    for entry in entries:
        if not entry.is_dir():
            continue
        if entry.name.startswith(".tmp-install-"):
            continue
        manifest = entry / "manifest.toml"
        if not manifest.is_file():
            continue
        data = _read_simple_toml(manifest)
        app_section = data.get("app") or {}
        app_id = str(app_section.get("id", entry.name))
        version = str(app_section.get("version", ""))
        # Source heuristic: presence of `.git/` means git checkout.
        if (entry / ".git").exists():
            source = "git"
        else:
            source = "local"
        try:
            caps = parse_capabilities_from_manifest_text(manifest.read_text())
        except OSError:
            caps = []
        rows.append(
            InstalledRow(
                id=app_id,
                version=version,
                source=source,
                capabilities=caps,
                path=str(entry),
            )
        )
    return rows


# ── Host CLI shell-out ───────────────────────────────────────────────────────
def _channel_binary_name() -> str:
    """Pick the right plexi binary to shell out to, matching this app's
    install channel. The host's binary lives next to its config dir;
    we infer the channel from the parent of `apps/`.
    Falls back to `plexi` for unsupported layouts (e.g. tests)."""
    apps_dir = _channel_apps_dir()
    if apps_dir is None:
        return "plexi"
    parent = apps_dir.parent.name  # ".plexi-alpha" / ".plexi-beta" / ".plexi"
    if parent == ".plexi":
        return "plexi"
    if parent.startswith(".plexi-"):
        return f"plexi-{parent.removeprefix('.plexi-')}"
    return "plexi"


def run_host_cli(
    args: list[str],
    *,
    binary_override: Optional[str] = None,
    timeout: float = 120.0,
) -> tuple[int, str, str]:
    """Run `plexi-<channel> <args...>` and capture (exit, stdout, stderr).

    `binary_override` is for tests; production picks the right binary
    automatically from this app's install location.
    """
    binary = binary_override or _channel_binary_name()
    resolved = shutil.which(binary) or binary
    cmd = [resolved, *args]
    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
        return result.returncode, result.stdout, result.stderr
    except FileNotFoundError as e:
        return 127, "", f"binary not found: {e}"
    except subprocess.TimeoutExpired:
        return 124, "", f"timed out after {timeout}s"
    except OSError as e:
        return 1, "", f"shell error: {e}"


# ── Registry fetch ───────────────────────────────────────────────────────────
@dataclass
class RegistryEntry:
    id: str
    name: str
    description: str
    source: str  # source-spec string, e.g. "github:owner/repo"
    version: str = ""
    capabilities: list[str] = field(default_factory=list)


def fetch_registry(
    url: str = DEFAULT_REGISTRY_URL, *, timeout: float = 5.0
) -> tuple[Optional[list[RegistryEntry]], Optional[str]]:
    """Fetch + parse the curated registry index. Returns
    (entries, None) on success or (None, error_message) on any failure
    — 404, DNS, malformed JSON, anything else.

    v3.2: the registry repo doesn't exist yet, so this is expected to
    fail with 404 in production. The caller surfaces an empty-state
    that points the user at the Install-by-URL tab.
    """
    try:
        with urllib.request.urlopen(url, timeout=timeout) as resp:
            raw = resp.read().decode("utf-8")
    except urllib.error.HTTPError as e:
        return None, f"registry returned HTTP {e.code}"
    except urllib.error.URLError as e:
        return None, f"network error: {e.reason}"
    except (TimeoutError, OSError) as e:
        return None, f"connection failed: {e}"
    try:
        data = json.loads(raw)
    except json.JSONDecodeError as e:
        return None, f"registry JSON invalid: {e}"
    apps_field = data.get("apps") if isinstance(data, dict) else None
    if not isinstance(apps_field, list):
        return None, "registry has no 'apps' array"
    entries: list[RegistryEntry] = []
    for raw_entry in apps_field:
        if not isinstance(raw_entry, dict):
            continue
        try:
            entries.append(
                RegistryEntry(
                    id=str(raw_entry["id"]),
                    name=str(raw_entry.get("name", raw_entry["id"])),
                    description=str(raw_entry.get("description", "")),
                    source=str(raw_entry["source"]),
                    version=str(raw_entry.get("version", "")),
                    capabilities=[
                        str(c) for c in raw_entry.get("capabilities", [])
                    ],
                )
            )
        except (KeyError, TypeError):
            continue
    return entries, None


# ── Install by URL: clone-to-temp + read manifest ────────────────────────────
def fetch_remote_manifest(
    source_spec: str,
) -> tuple[Optional[dict], Optional[str]]:
    """Shallow-clone the source into a temp dir and read its manifest.

    Returns (manifest_dict, None) on success or (None, error_message).

    `manifest_dict` carries `id`, `name`, `version`, `description`, and
    `capabilities` — flattened for the preview UI's convenience.
    """
    # Translate `github:owner/repo` → real URL the same way the host does.
    url, git_ref = _translate_source_to_url(source_spec)
    if url is None:
        return None, f"unknown source scheme: {source_spec}"

    import tempfile

    tmpdir = tempfile.mkdtemp(prefix="app-store-preview-")
    try:
        clone = [
            "git",
            "clone",
            "--depth",
            "1",
            "--quiet",
        ]
        if git_ref:
            clone.extend(["--branch", git_ref])
        clone.extend([url, tmpdir])
        result = subprocess.run(
            clone, capture_output=True, text=True, timeout=30, check=False
        )
        if result.returncode != 0:
            return None, f"git clone failed: {result.stderr.strip()}"
        manifest_path = Path(tmpdir) / "manifest.toml"
        if not manifest_path.is_file():
            return None, "repo has no manifest.toml at root"
        text = manifest_path.read_text()
        data = _read_simple_toml(manifest_path)
        app_section = data.get("app") or {}
        return (
            {
                "id": str(app_section.get("id", "")),
                "name": str(app_section.get("name", app_section.get("id", ""))),
                "version": str(app_section.get("version", "")),
                "description": str(app_section.get("description", "")),
                "capabilities": parse_capabilities_from_manifest_text(text),
                "schema_version": str(data.get("schema_version", "")),
            },
            None,
        )
    except subprocess.TimeoutExpired:
        return None, "git clone timed out"
    except OSError as e:
        return None, f"clone error: {e}"
    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)


def _translate_source_to_url(spec: str) -> tuple[Optional[str], Optional[str]]:
    """`github:owner/repo[@ref]` → `(https://github.com/owner/repo.git, ref)`.
    `git+https://...[@ref]` → `(https://..., ref)`.
    Returns `(None, None)` for unknown schemes."""
    # Split optional `@ref` first (matches host `split_source_and_ref`).
    src = spec
    git_ref: Optional[str] = None
    # The `@` may appear in `git+ssh://user@host/...` — only treat the LAST `@`
    # as a ref delimiter if there's a `/` after the scheme separator.
    scheme_end = spec.find("://")
    scheme_end = scheme_end + 3 if scheme_end >= 0 else 0
    after = spec[scheme_end:]
    if "@" in after:
        # Find the last `@` after the scheme.
        idx = spec.rfind("@")
        if idx >= scheme_end:
            tail = spec[scheme_end:idx]
            if "/" in tail or "/" not in after:
                src = spec[:idx]
                ref_candidate = spec[idx + 1 :]
                if ref_candidate:
                    git_ref = ref_candidate
    if src.startswith("github:"):
        rest = src[len("github:") :].rstrip("/").removesuffix(".git")
        if "/" not in rest or rest.startswith("/"):
            return None, None
        return f"https://github.com/{rest}.git", git_ref
    if src.startswith("git+"):
        return src[len("git+") :], git_ref
    return None, None


# ── App ──────────────────────────────────────────────────────────────────────
class AppStoreApp(App):
    VIEW_BROWSE = "browse"
    VIEW_URL = "url"
    VIEW_INSTALLED = "installed"

    def on_init(self, ctx: RenderContext) -> None:
        self._view: str = self.VIEW_BROWSE
        self._registry: Optional[list[RegistryEntry]] = None
        self._registry_error: Optional[str] = None
        self._registry_loading: bool = False
        self._installed: list[InstalledRow] = []
        self._action_log: list[str] = []  # last N action results
        self._busy: bool = False  # True while a shell-out is in flight
        self._scrollable_browse = Scrollable(child=Column([]))
        self._scrollable_installed = Scrollable(child=Column([]))
        self._reload_installed()
        self._refresh_registry_async()
        self.emit.info("App Store started")

    # ── State helpers ─────────────────────────────────────────────────────
    def _reload_installed(self) -> None:
        try:
            self._installed = scan_installed_apps()
        except OSError as e:
            self._installed = []
            self.emit.warn(f"app-store: could not scan installed apps: {e}")

    def _log_action(self, line: str) -> None:
        # Keep the last 10 lines of action output for the in-app status panel.
        self._action_log.append(line)
        if len(self._action_log) > 10:
            self._action_log = self._action_log[-10:]

    def _refresh_registry_async(self) -> None:
        if self._registry_loading:
            return
        self._registry_loading = True
        self._registry_error = None
        self.emit.schedule_render(after_ms=20)

        def worker() -> None:
            entries, err = fetch_registry()
            self._registry = entries
            self._registry_error = err
            self._registry_loading = False
            self.emit.schedule_render(after_ms=20)

        threading.Thread(target=worker, daemon=True).start()

    # ── Capability preview modal ──────────────────────────────────────────
    def _confirm_capabilities(
        self, app_id: str, source: str, capabilities: list[str]
    ) -> bool:
        """Show the capability preview as a notify_choice modal.

        Returns True if the user confirmed install, False on cancel.
        Mandatory before any install — see #308 Phase 3 spec.
        """
        if capabilities:
            cap_lines = "\n".join(f"  • {capability_label(c)}" for c in capabilities)
        else:
            cap_lines = "  (no capabilities declared)"
        body = (
            f"Source: {source}\n\n"
            f"This app declares the following capabilities:\n{cap_lines}\n\n"
            "Capabilities are gated at install AND runtime — you can revoke "
            "them later via the Permissions menu."
        )
        choice = self.emit.run_sync(self.emit.notify_choice(
            title=f"Install {app_id}?",
            body=body,
            options=[
                {"label": "Install", "value": "install", "shortcut": "i"},
                {"label": "Cancel", "value": "cancel", "shortcut": "c"},
            ],
            level="info",
            required=False,
            priority=100,
        ))
        return choice == "install"

    # ── Install / uninstall / update / pack-export actions ────────────────
    def _install_action(self, source_spec: str, capabilities: list[str]) -> None:
        """Capability prompt → shell out to `plexi-<channel> install`.
        Runs on a background thread so the modal + subprocess don't stall
        the render loop. The capability prompt itself blocks; that's by
        design — the user has to make the call before anything else
        happens."""
        if self._busy:
            self._log_action("busy: another install in flight, try again")
            return

        def worker() -> None:
            self._busy = True
            try:
                ok = self._confirm_capabilities(source_spec, source_spec, capabilities)
                if not ok:
                    self._log_action(f"cancelled: {source_spec}")
                    return
                code, out, err = run_host_cli(["install", source_spec])
                if code == 0:
                    self._log_action(f"installed: {source_spec}")
                    self._reload_installed()
                else:
                    msg = (err or out or "unknown error").strip().splitlines()[-1]
                    self._log_action(f"install failed [{code}]: {msg}")
            finally:
                self._busy = False
                self.emit.schedule_render(after_ms=20)

        threading.Thread(target=worker, daemon=True).start()

    def _install_by_url_action(self, source_spec: str) -> None:
        """Fetch the manifest for capability preview, then install."""
        if self._busy:
            self._log_action("busy: another install in flight")
            return

        def worker() -> None:
            self._busy = True
            try:
                manifest, err = fetch_remote_manifest(source_spec)
                if manifest is None:
                    self._log_action(f"manifest fetch failed: {err}")
                    return
                ok = self._confirm_capabilities(
                    manifest.get("id") or source_spec,
                    source_spec,
                    list(manifest.get("capabilities") or []),
                )
                if not ok:
                    self._log_action(f"cancelled: {source_spec}")
                    return
                code, out, errstr = run_host_cli(["install", source_spec])
                if code == 0:
                    self._log_action(f"installed: {source_spec}")
                    self._reload_installed()
                else:
                    msg = (errstr or out or "unknown error").strip().splitlines()[-1]
                    self._log_action(f"install failed [{code}]: {msg}")
            finally:
                self._busy = False
                self.emit.schedule_render(after_ms=20)

        threading.Thread(target=worker, daemon=True).start()

    def _uninstall_action(self, app_id: str) -> None:
        if self._busy:
            return

        def worker() -> None:
            self._busy = True
            try:
                code, out, err = run_host_cli(["uninstall", app_id, "--yes"])
                if code == 0:
                    self._log_action(f"uninstalled: {app_id}")
                    self._reload_installed()
                else:
                    msg = (err or out or "unknown error").strip().splitlines()[-1]
                    self._log_action(f"uninstall failed [{code}]: {msg}")
            finally:
                self._busy = False
                self.emit.schedule_render(after_ms=20)

        threading.Thread(target=worker, daemon=True).start()

    def _update_action(self, app_id: Optional[str]) -> None:
        if self._busy:
            return

        def worker() -> None:
            self._busy = True
            try:
                args = ["update"] if app_id is None else ["update", app_id]
                code, out, err = run_host_cli(args)
                label = "all" if app_id is None else app_id
                if code == 0:
                    self._log_action(f"updated: {label}")
                    self._reload_installed()
                else:
                    msg = (err or out or "unknown error").strip().splitlines()[-1]
                    self._log_action(f"update failed [{code}]: {msg}")
            finally:
                self._busy = False
                self.emit.schedule_render(after_ms=20)

        threading.Thread(target=worker, daemon=True).start()

    def _export_pack_action(self) -> None:
        """Prompt for a destination path, then shell out to
        `plexi-<channel> pack export <path>`."""
        if self._busy:
            return

        def worker() -> None:
            self._busy = True
            try:
                default_dest = str(Path.home() / "plexi-installed.pack.toml")
                dest = self.emit.run_sync(self.emit.notify_input(
                    title="Export installed pack",
                    prompt="Destination path",
                    body=(
                        "Writes a pack.toml describing every installed app. "
                        "Re-apply with `plexi install --pack <path>`.\n\n"
                        f"Default: {default_dest}"
                    ),
                    required=False,
                    priority=80,
                ))
                if dest == "__cancel__":
                    self._log_action("export cancelled")
                    return
                if not dest:
                    dest = default_dest
                code, out, err = run_host_cli(["pack", "export", dest])
                if code == 0:
                    self._log_action(f"exported → {dest}")
                else:
                    msg = (err or out or "unknown error").strip().splitlines()[-1]
                    self._log_action(f"export failed [{code}]: {msg}")
            finally:
                self._busy = False
                self.emit.schedule_render(after_ms=20)

        threading.Thread(target=worker, daemon=True).start()

    # ── Keys ──────────────────────────────────────────────────────────────
    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        # Tab switching first (always available).
        if key == "1":
            self._view = self.VIEW_BROWSE
            return
        if key == "2":
            self._view = self.VIEW_URL
            return
        if key == "3":
            self._view = self.VIEW_INSTALLED
            return

        # View-local scroll routing.
        if self._view == self.VIEW_BROWSE:
            if self._scrollable_browse.handle_key(key):
                return
        elif self._view == self.VIEW_INSTALLED:
            if self._scrollable_installed.handle_key(key):
                return

        # View-local actions.
        if self._view == self.VIEW_BROWSE:
            if key.lower() == "r":
                self._refresh_registry_async()
                return
            if key.lower() == "i":
                # Install the first not-yet-installed entry. Real per-row
                # selection is a follow-up; for v3.2 the registry is empty
                # anyway so this is a forward-compatible affordance.
                self._install_first_available()
                return
        if self._view == self.VIEW_INSTALLED:
            if key.lower() == "r":
                self._reload_installed()
                return
            if key.lower() == "u":
                self._update_action(None)
                return
            if key.lower() == "e":
                self._export_pack_action()
                return

    def _install_first_available(self) -> None:
        if not self._registry:
            return
        installed_ids = {a.id for a in self._installed}
        for entry in self._registry:
            if entry.id in installed_ids:
                continue
            self._install_action(entry.source, entry.capabilities)
            return
        self._log_action("nothing to install: all registry apps already installed")

    # ── Render ────────────────────────────────────────────────────────────
    def on_render(self, ctx: RenderContext) -> None:
        tabs = self._render_tab_bar()
        body: list = []
        if self._view == self.VIEW_BROWSE:
            body = self._render_browse()
        elif self._view == self.VIEW_URL:
            body = self._render_url(ctx)
        else:
            body = self._render_installed()

        action_log_rows: list = []
        if self._action_log:
            for line in self._action_log[-5:]:
                action_log_rows.append(Label(line))
        else:
            action_log_rows.append(Label("(no actions yet)"))

        ctx.render(Column([
            AppBar(title="App Store"),
            Label(tabs),
            *body,
            Section("Recent actions"),
            Card(action_log_rows),
            Spacer(grow=True),
            Footer(
                "1 Browse  2 Install by URL  3 Installed  "
                "j/k scroll  r refresh"
            ),
        ]))

    def _render_tab_bar(self) -> str:
        marker = lambda v: "●" if self._view == v else "○"  # noqa: E731
        return (
            f"{marker(self.VIEW_BROWSE)} 1 Browse   "
            f"{marker(self.VIEW_URL)} 2 Install by URL   "
            f"{marker(self.VIEW_INSTALLED)} 3 Installed"
        )

    def _render_browse(self) -> list:
        rows: list = []
        if self._registry_loading:
            rows.append(Label("Loading registry…"))
            rows.append(Card([Label("Fetching curated app list over HTTP")]))
            return rows
        if self._registry_error:
            rows.append(Section("Registry unavailable"))
            rows.append(Card([
                Label(f"Error: {self._registry_error}"),
                Label(""),
                Label(
                    "The curated registry repo doesn't exist yet for v3.2. "
                    "Switch to the Install by URL tab (press 2) to install "
                    "any public Plexi app from a git URL."
                ),
                KeyRow("r", "Retry registry fetch"),
                KeyRow("2", "Switch to Install by URL"),
            ]))
            return rows
        entries = self._registry or []
        if not entries:
            rows.append(Section("Registry empty"))
            rows.append(Card([Label("No apps in the registry index.")]))
            return rows

        installed_ids = {a.id for a in self._installed}
        list_rows: list = [Label(f"{len(entries)} apps available")]
        for entry in entries:
            badge = "[Installed ✓]" if entry.id in installed_ids else "[Install]"
            cap_summary = ", ".join(entry.capabilities) or "(no caps)"
            list_rows.append(Label(f"{badge} {entry.id}  v{entry.version}"))
            list_rows.append(Label(f"  {entry.description}"))
            list_rows.append(Label(f"  source: {entry.source}"))
            list_rows.append(Label(f"  caps:   {cap_summary}"))
            list_rows.append(Label(""))

        self._scrollable_browse.child = Column(list_rows)
        rows.append(Section("Browse registry"))
        rows.append(self._scrollable_browse)
        rows.append(Card([
            KeyRow("i", "Install first un-installed entry"),
            KeyRow("r", "Refresh registry"),
        ]))
        return rows

    def _render_url(self, ctx: RenderContext) -> list:
        # Single text input; on submit, fetch the manifest and run the
        # capability preview + install flow on a background thread.
        # We render the input via DrawCommand::TextInput at a fixed pixel
        # rect — the Card layout above it gives the user something to read
        # while typing.
        rows: list = []
        rows.append(Section("Install by URL"))
        rows.append(Card([
            Label(
                "Paste a git URL or `github:owner/repo[@ref]`. The app fetches "
                "the manifest, shows the capability preview, then installs."
            ),
            Label("Examples:"),
            Label("  github:plexi-apps/example-app"),
            Label("  github:plexi-apps/example-app@v1.2.0"),
            Label("  git+https://example.com/some-app.git"),
        ]))
        # Reserve a band for the input. ctx.text_input handles the focus,
        # caret, and Enter-to-submit — the host owns the buffer.
        input_y = 200.0  # below the AppBar + tabs + intro card
        submitted = ctx.text_input(
            "app-store-url",
            x=20.0,
            y=input_y,
            w=ctx.width - 40.0 if hasattr(ctx, "width") else 600.0,
            placeholder="github:owner/repo[@ref]  or  git+https://…  (Enter to install)",
        )
        if submitted is not None:
            submitted = submitted.strip()
            if submitted:
                self._install_by_url_action(submitted)
            else:
                self._log_action("ignored: empty URL")
        rows.append(Spacer(grow=False))
        # Pad with an empty band so the Card under the input doesn't render
        # over the host-painted text input.
        rows.append(Label(""))
        rows.append(Label(""))
        rows.append(Label(""))
        return rows

    def _render_installed(self) -> list:
        rows: list = []
        if not self._installed:
            rows.append(Section("No apps installed"))
            rows.append(Card([
                Label("Nothing in ~/.plexi-<channel>/apps/."),
                Label("Switch to Browse (1) or Install by URL (2)."),
            ]))
            return rows

        list_rows: list = [Label(f"{len(self._installed)} apps installed")]
        for app in self._installed:
            badge = {"git": "[git]", "local": "[local]"}.get(app.source, "[?]")
            list_rows.append(Label(
                f"{badge} {app.id:30} v{app.version:12} {len(app.capabilities)} caps"
            ))
        self._scrollable_installed.child = Column(list_rows)
        rows.append(Section("Installed apps"))
        rows.append(self._scrollable_installed)
        rows.append(Card([
            KeyRow("u", "Update all"),
            KeyRow("e", "Export current setup → pack.toml"),
            KeyRow("r", "Reload from disk"),
        ]))
        return rows


if __name__ == "__main__":
    AppStoreApp().run()
