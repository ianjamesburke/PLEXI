# plexi-sdk

Python SDK v2 for building Plexi PGAP apps.

SDK v2 is the Python authoring API. PGAP v3 (`pgap/3`) is the host/app wire protocol the SDK speaks.

Full API reference: `plexi_sdk/__init__.py` and `docs/sdk-v2.md`.

## Install For SDK Development

```sh
uv pip install -e ./sdk/python
```

Run from the repo root. `uv` creates `.venv` automatically if needed. Changes to `sdk/python/plexi_sdk/` take effect immediately in that environment.

Apps running inside Plexi use the SDK copy injected by the host on `PYTHONPATH`; the editable install is for type checking, tests, and local development.

Verify the install:

```sh
source .venv/bin/activate
python3 -c "from plexi_sdk import App; print('ok')"
```

## App Pattern

Normal apps implement `view()` and return a component tree:

```python
from plexi_sdk import App
from plexi_sdk.ui import AppBar, Column, FooterKeys, Label, Spacer


class CounterApp(App):
    def on_init(self) -> None:
        self.count = self.state.get("count", 0)

    def view(self):
        return Column([
            AppBar("Counter"),
            Spacer(grow=True),
            Label(str(self.count), bold=True),
            Spacer(grow=True),
            FooterKeys([("+", "increment"), ("-", "decrement")]),
        ])

    def on_key(self, key: str, mods: dict) -> None:
        if key in ("plus", "equals"):
            self.count += 1
        elif key == "minus":
            self.count -= 1
        self.state.save({"count": self.count})
        self.emit.schedule_render()


CounterApp().run()
```

Use `on_render(ctx)` only for games, animations, realtime visualizations, or other pixel-control apps.

## Keyboard Conventions

Keys arrive in `on_key(self, key, mods)` as lowercase canonical strings.

| Physical key | `key` string |
|---|---|
| Enter / Return | `"return"` |
| Escape | `"escape"` |
| Backspace | `"backspace"` |
| Tab | `"tab"` |
| Space | `"space"` |
| Arrow keys | `"up"` / `"down"` / `"left"` / `"right"` |

Use `"return"`, not `"Enter"`. Use `"escape"`, not `"Escape"`.

Common list bindings:

| Key | Action |
|---|---|
| `j` / `"down"` | Move selection down |
| `k` / `"up"` | Move selection up |
| `"return"` | Open selected item |
| `"escape"` | Exit list or go back |

## State And Host I/O

```python
self.state.get("key", default)
self.state.save({"key": value})
self.emit.schedule_render()
await self.emit.http_get(url)
await self.emit.secret_get("API_KEY")
await self.emit.ai_query("low", system, messages)
```

Declare capabilities in `manifest.toml` before using brokered host powers.
