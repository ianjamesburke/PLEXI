# plexi-sdk

Python SDK for building Plexi apps via the PGAP v3 protocol.

Full API reference: `plexi_sdk/__init__.py` docstring.

---

## Keyboard Conventions

Apps should follow these standard navigation bindings so users get consistent behavior across all Plexi apps.

### Key name strings

Keys arrive in `on_key(ctx, key, mods)` as **lowercase canonical strings**:

| Physical key | `key` string |
|---|---|
| Enter / Return | `"return"` |
| Escape | `"escape"` |
| Backspace | `"backspace"` |
| Tab | `"tab"` |
| Space | `"space"` |
| Arrow keys | `"up"` / `"down"` / `"left"` / `"right"` |

The SDK normalizes egui's internal key names automatically. Always match against
the lowercase canonical strings — `key == "return"`, not `key == "Enter"`.

### Focus / modal conventions

| Action | Key |
|---|---|
| Open item / confirm / select | `"return"` |
| Exit focused view / go back | `"escape"` |
| Exit focused view (fallback) | `"backspace"` (optional second binding) |

Any view that opens a focused sub-view (detail panel, modal, inline editor) must
let the user exit with `"escape"`. No view should be a dead end.

**TextInput exception:** `"return"` fires `on_text_submitted`, so the TextInput
widget owns that key while focused. `"escape"` should still dismiss the
surrounding view or cancel the input mode.

```python
def on_key(self, ctx, key, mods):
    if self.detail_open:
        if key in ("escape", "backspace"):
            self.detail_open = False
            return
        if key == "return":
            self._confirm()
            return
    # list navigation below
```

### List navigation

Standard bindings for any scrollable list:

| Key | Action |
|---|---|
| `j` / `"down"` | Move selection down |
| `k` / `"up"` | Move selection up |
| `"return"` | Open selected item |
| `"escape"` | Exit list / go back |

---

## List + Detail Views

For any app with a list-and-detail pattern, prefer `SelectList` from
`plexi_sdk.ui` over managing selection state manually. It handles j/k/arrow
navigation, scroll-to-keep-selected-visible, and click hit-testing.

```python
from plexi_sdk import App
from plexi_sdk.ui import SelectList, Column

class MyApp(App):
    async def on_init(self, ctx):
        self.items = [{"name": "Alpha"}, {"name": "Beta"}, {"name": "Gamma"}]
        self.list = SelectList(self.items)  # stateful — create once, not per frame
        self.detail_open = False

    def on_key(self, ctx, key, mods):
        if self.detail_open:
            if key in ("escape", "backspace"):
                self.detail_open = False
            return
        if key == "return":
            self.detail_open = True
            return
        self.list.handle_key(key)  # handles j/k/up/down

    def on_render(self, ctx):
        if self.detail_open:
            ctx.text(20, 40, self.items[self.list.selected_idx]["name"])
        else:
            ctx.draw(Column([self.list.render(self.items)]))
```

`SelectList` is stateful — create it in `on_init`, not `on_render`.

For the lower-level `ctx.list_view()` primitive (draw only, no state) see the
`ListItem` shape in the API reference.
