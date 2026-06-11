#!/usr/bin/env python3
"""Permissions — first-party permission manager (stint 0017).

Lists every (app, workspace, capability) permission row the host knows
about via `list_permissions`, grouped by app, and lets the user flip a
row to Allow (green) / Ask (yellow) / Block (red) via `set_permission`.
Both calls gate on the sensitive `permissions.manage` capability.
"""
from plexi_sdk import App, CapabilityDeniedError
from plexi_sdk.ui import AppBar, Column, FooterKeys, Label, SelectList, Spacer

_STATE_BADGE = {"green": "● allow", "yellow": "● ask", "red": "● block"}


class Permissions(App):
    def on_init(self) -> None:
        self._rows: list[dict] = []
        self._running: list[str] = []
        self._loading = True
        self._denied = False
        self._error: str | None = None
        self._list = SelectList([])
        self.emit.info("Permissions app initialized — fetching permission rows")
        self.emit.schedule_task(self._refresh())

    # ── Data ──────────────────────────────────────────────────────────────────

    async def _refresh(self) -> None:
        """Fetch permission rows from the host and rebuild the list."""
        self._loading = True
        self._error = None
        self.emit.schedule_render()
        try:
            data = await self.emit.list_permissions()
            rows = data.get("permissions", [])
            # Group by app: stable sort by (app_id, capability).
            rows.sort(key=lambda r: (r.get("app_id", ""), r.get("capability", "")))
            self._rows = rows
            self._running = data.get("running", [])
            self._denied = False
            self.emit.info(f"Permissions: fetched {len(rows)} rows "
                           f"({len(self._running)} running apps)")
        except CapabilityDeniedError as exc:
            self._denied = True
            self.emit.warn(f"Permissions: list_permissions denied: {exc}")
        except Exception as exc:
            self._error = str(exc)
            self.emit.error(f"Permissions: list_permissions failed: {exc}")
        finally:
            self._loading = False
            self._rebuild_list()
            self.emit.schedule_render()

    def _rebuild_list(self) -> None:
        items = []
        prev_app = None
        for row in self._rows:
            app_id = row.get("app_id", "?")
            badge = _STATE_BADGE.get(row.get("state", ""), row.get("state", "?"))
            running = " · running" if app_id in self._running else ""
            sensitive = " · [sensitive]" if row.get("sensitive") else ""
            stored = "" if row.get("stored", True) else " · live"
            group = app_id if app_id != prev_app else " " * len(app_id)
            prev_app = app_id
            items.append({
                "name": f"{group}  {row.get('capability', '?')}",
                "description": (row.get("description") or "") + running,
                "trailing": f"{badge}{sensitive}{stored}",
            })
        selected = min(self._list.selected_idx, max(0, len(items) - 1))
        self._list = SelectList(items, selected_idx=selected)

    # ── Mutations ─────────────────────────────────────────────────────────────

    async def _set_selected(self, state: str) -> None:
        if not self._rows:
            return
        row = self._rows[self._list.selected_idx]
        app_id = row.get("app_id", "")
        capability = row.get("capability", "")
        workspace = row.get("workspace")
        self.emit.info(f"Permissions: set {app_id}/{capability} "
                       f"@ {workspace} → {state}")
        try:
            await self.emit.set_permission(
                app_id, capability, state, workspace=workspace)
        except CapabilityDeniedError as exc:
            self._denied = True
            self.emit.warn(f"Permissions: set_permission denied: {exc}")
            self.emit.schedule_render()
            return
        except Exception as exc:
            self._error = str(exc)
            self.emit.error(f"Permissions: set_permission failed: {exc}")
            self.emit.schedule_render()
            return
        await self._refresh()

    async def _retry_after_denial(self) -> None:
        """Re-ask for permissions.manage via the host grant modal, then refetch."""
        try:
            await self.emit.capability_request("permissions.manage")
        except CapabilityDeniedError:
            self.emit.warn("Permissions: user denied permissions.manage again")
            self.emit.schedule_render()
            return
        self._denied = False
        await self._refresh()

    # ── View ──────────────────────────────────────────────────────────────────

    def view(self):
        if self._denied:
            body: list = [
                Spacer(grow=True),
                Label("permissions.manage was denied — grant it to manage "
                      "app permissions", tone="hint"),
                Spacer(grow=True),
            ]
            keys = [("r", "retry"), ("esc", "close")]
        elif self._loading:
            body = [Spacer(grow=True),
                    Label("Loading permissions…", tone="hint"),
                    Spacer(grow=True)]
            keys = [("esc", "close")]
        else:
            body = []
            if self._error:
                body.append(Label(f"Error: {self._error}", tone="hint"))
            body.append(self._list)
            keys = [("j/k", "select"), ("g", "allow"), ("y", "ask"),
                    ("b", "block"), ("r", "refresh"), ("esc", "close")]
        return Column([
            AppBar(title="Permissions",
                   subtitle=f"{len(self._rows)} rows"),
            *body,
            FooterKeys(shortcuts=keys),
        ])

    # ── Keys ──────────────────────────────────────────────────────────────────

    def on_key(self, key: str, _mods: dict) -> None:
        if self._denied:
            if key == "r":
                self.emit.schedule_task(self._retry_after_denial())
            return
        if self._list.handle_key(key):
            self.emit.schedule_render()
            return
        if key == "r":
            self.emit.schedule_task(self._refresh())
        elif key == "g":
            self.emit.schedule_task(self._set_selected("green"))
        elif key == "y":
            self.emit.schedule_task(self._set_selected("yellow"))
        elif key == "b":
            self.emit.schedule_task(self._set_selected("red"))


if __name__ == "__main__":
    Permissions().run()
