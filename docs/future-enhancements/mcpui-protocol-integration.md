# MCPUI Protocol Integration

**Issue:** #2056
**Status:** Future enhancement — P4, not scheduled
**Spec:** https://modelcontextprotocol.io/extensions/apps/overview (SEP-1865, 2026-01-26)

---

## What MCPUI Is

MCP Apps (MCPUI) is an official MCP extension protocol where:

- An **MCP server** returns a `ui://` resource from a tool call — an HTML file with MIME type `text/html;profile=mcp-app`
- The **MCP host** fetches that resource, renders it in a sandboxed iframe (web) or WebView (native), and injects a postMessage bridge
- The **app** (running in the WebView) and host communicate via **JSON-RPC 2.0 over postMessage** using `ui/` method prefixes
- The app can call MCP tools back through the host (`tools/call`); the host can push tool results to the app (`ui/notifications/tool-result`)

This is structurally different from Plexi's PGAP model. PGAP apps emit egui draw commands over stdio. MCPUI apps render HTML and speak postMessage. These are parallel runtimes, not converging ones.

Current ecosystem support: Claude Desktop, VS Code Copilot, Goose, MCPJam, Archestra.AI.

---

## Two Directions

### Direction B — Plexi apps as MCPUI servers (easy, ship first)

A PGAP app's HTTP MCP bridge (`mcp_server.rs`) also serves a `ui://` resource. External hosts (Claude Desktop, etc.) can fetch it and render it. Plexi itself doesn't render HTML.

**What this unlocks:** A Plexi app author ships one binary (Python/PGAP) + one HTML file. The app works inside Plexi natively and inside Claude Desktop as an MCPUI app. Same MCP tool surface, two rendering paths.

**Cost:** ~200–300 lines of changes to `mcp_server.rs` and `registry.rs`. No new pane type. No WebView.

### Direction A — Plexi as MCPUI host (hard, ship second)

Plexi renders MCPUI apps in a new `Pane::WebView` variant. A WKWebView (macOS) is embedded alongside the egui/wgpu surface, bridges postMessage over a native script handler, and speaks the full `ui/` protocol.

**What this unlocks:** Any MCPUI-compatible app (not just Plexi apps) can open as a pane. True ecosystem parity.

**Cost:** Multi-week infrastructure effort. New crate dependency (`wry`), new pane type, MCP client, coordinate system work. Detailed below.

---

## Direction B: Implementation Map

### Manifest changes (`src/app/registry.rs`)

Add `McpUiSection` alongside the existing `McpSection`:

```toml
[app.mcpui]
resource_uri = "ui://my-app/index.html"
html_path = "bin/index.html"   # relative to app dir

[app.mcpui.csp]
connect_domains = ["api.example.com"]
resource_domains = ["cdn.example.com"]
```

```rust
// src/app/registry.rs — add alongside McpSection
#[derive(Deserialize, Debug, Clone)]
pub struct McpUiSection {
    pub resource_uri: String,
    pub html_path: PathBuf,
    #[serde(default)]
    pub csp: Option<McpUiCsp>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct McpUiCsp {
    #[serde(default)] pub connect_domains: Vec<String>,
    #[serde(default)] pub resource_domains: Vec<String>,
    #[serde(default)] pub frame_domains: Vec<String>,
}

// Add to AppManifestApp:
#[serde(default)]
pub mcpui: Option<McpUiSection>,
```

### MCP bridge changes (`src/process_app/mcp_server.rs`)

1. Advertise `"resources": {}` in the `initialize` capabilities response (currently only advertises `"tools": {}`).

2. Handle `resources/list` — return the `ui://` resource with correct MIME type and `_meta.ui.csp`:
```json
{
  "uri": "ui://my-app/index.html",
  "name": "My App",
  "mimeType": "text/html;profile=mcp-app",
  "_meta": { "ui": { "csp": { ... } } }
}
```

3. Handle `resources/read` — read the HTML file from `<app_dir>/<html_path>`, return as `"text"` content with the same metadata.

4. Pass `McpUiSection` into `start_mcp_server` so the handler has access to the file path and CSP config.

### Capability negotiation

During MCP `initialize`, the host must advertise extension support:
```json
{
  "capabilities": {
    "extensions": {
      "io.modelcontextprotocol/ui": {
        "mimeTypes": ["text/html;profile=mcp-app"]
      }
    }
  }
}
```

This is added to the `initialize` response body in `handle_connection`.

### Do not touch

- PGAP stdio path — untouched
- `src/app/lifecycle.rs` — untouched for Direction B
- `src/host/pane.rs` — untouched
- Any egui/wgpu render code

---

## Direction A: WebView Pane — Full Technical Breakdown

### The Core Problem: Two GPU Subsystems

Plexi today: eframe → winit → wgpu → Metal (macOS). Every pane is a region of a single Metal command buffer. There is no WebView crate in `Cargo.toml`.

WKWebView is an `NSView`-backed Cocoa component. It lives in the Cocoa view hierarchy, parallel to (not inside) the Metal surface. You cannot paint a WKWebView into a wgpu texture. These are fundamentally different display subsystems.

**The solution is overlay positioning, not embedding.** The WebView NSView floats above the Metal surface, repositioned every frame to match the pane's screen rect. From the user's perspective it looks embedded. This is the same technique VS Code uses for WebView panels.

### egui vs Tauri — Does It Matter?

The user asked: does not using Tauri complicate this?

**No. It simplifies it.** Tauri wraps `wry` in application lifecycle management you don't need. `wry` is a standalone Rust crate that wraps WKWebView directly. It works with any winit-based app — which is what eframe is under the hood. You use `wry` directly, pass it a `raw-window-handle` from the winit window, and skip Tauri entirely.

The complication is not Tauri vs egui. The complication is Metal surface + NSView coexistence, which is the same regardless of framework.

```toml
# Cargo.toml addition
[target.'cfg(target_os = "macos")'.dependencies]
wry = { version = "0.46", default-features = false, features = ["mac-proxy"] }
```

### New Pane Variant

`src/host/pane.rs` — add `Pane::WebView`:

```rust
pub enum Pane {
    Terminal(Box<TerminalPane>),
    App(Box<AppPane>),
    Portal(Box<PortalPane>),
    WebView(Box<WebViewPane>),  // new
}
```

`WebViewPane` fields:
- `id: PaneId`
- `name: Option<String>`
- `hidden: bool`
- `webview: wry::WebView` — the live WKWebView instance
- `bridge: McpUiBridge` — owns JS↔Rust channels
- `mcp_client: McpUiClient` — outbound MCP client to the server that provided the resource
- `display_mode: McpUiDisplayMode` — inline | fullscreen | pip
- `resource_uri: String` — the `ui://` URI this pane is rendering
- `initialized: bool` — whether `ui/notifications/initialized` has been received

### WebView Creation

WKWebView must be created on the main thread. The right place in eframe is the `setup` callback (if available in 0.31) or the first frame using a `OnceCell<WebView>`.

```rust
// Inside the host's main update loop, first frame only:
let raw_handle = frame.raw_window_handle(); // eframe 0.31 API
let webview = WebViewBuilder::new()
    .with_html(html_content)
    .with_initialization_script(BRIDGE_SHIM_JS)
    .with_ipc_handler(move |msg| { tx.send(msg).ok(); })
    .build_as_child(&raw_handle)?;
webview.set_bounds(wry::Rect { x, y, width, height });
```

### Coordinate Positioning (Every Frame)

Each frame, after egui lays out tiles, for each WebView pane:

1. Get the pane's rect from the tile layout in logical pixels.
2. Multiply by `NSScreen.backingScaleFactor` (NOT egui's `pixels_per_point` — they may differ).
3. Call `webview.set_bounds()`.
4. If any modal is open (`host_state.modal_open`), call `webview.set_visible(false)` to prevent drawing over overlays.

**Critical:** Use `NSScreen.backingScaleFactor` for WKWebView positioning. egui's `pixels_per_point` and NSScreen's scale factor are not guaranteed to match. Using egui's value produces misaligned WebViews on non-standard DPI configs.

### postMessage Bridge — Native vs Web

The spec's two-iframe sandbox proxy pattern is for **web hosts only**. For a native macOS host, skip it entirely. WKWebView has native sandboxing via `WKWebViewConfiguration` and `WKContentRuleList`.

**JS → Rust:** The app calls `window.webkit.messageHandlers.mcpBridge.postMessage(json)`. This fires `WKScriptMessageHandler` on the Rust side (main thread). The message goes into `McpUiBridge.inbound`.

**Rust → JS:** The host calls `webview.evaluate_script("window.__mcpBridge.receive(" + json + ")")`.

**The SDK shim problem:** `@modelcontextprotocol/ext-apps` uses `window.parent.postMessage()` internally (designed for web iframes). In WKWebView there's no parent frame. You must inject a shim before page load:

```javascript
// Injected as WKUserScript, runs before page content
(function() {
  // Intercept the SDK's postMessage transport
  const _origPostMessage = window.parent ? window.parent.postMessage.bind(window.parent) : null;
  window.__mcpParentPostMessage = function(msg, origin) {
    window.webkit.messageHandlers.mcpBridge.postMessage(
      typeof msg === 'string' ? msg : JSON.stringify(msg)
    );
  };
  if (window.parent && window.parent !== window) {
    window.parent.postMessage = window.__mcpParentPostMessage;
  }
  // Receive from host
  window.__mcpBridge = {
    receive: function(json) {
      window.dispatchEvent(new MessageEvent('message', {
        data: json, origin: 'plexi://host', source: window.parent
      }));
    }
  };
})();
```

### CSP Enforcement (Native)

For a native host, CSP is applied two ways:

1. **Meta tag in HTML:** Already handled if the app author includes it.
2. **WKContentRuleList:** Plexi builds a content blocking rule list from the manifest's `[app.mcpui.csp]` declarations and installs it on `WKWebViewConfiguration`. This enforces CSP even if the app omits the meta tag. Undeclared `connect_domains` → blocked.

Restrictive defaults (if `csp` section omitted):
```
default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline';
img-src 'self' data:; connect-src 'none'; object-src 'none';
```

### `plexi app action` Integration

Current dispatch path in `src/app/lifecycle.rs:527`:
```rust
// PGAP path:
app_pane.runtime.queue_outbound_event(PlexiEvent::Action { action, args })
```

For WebView panes, add a parallel branch:
```rust
// MCPUI path:
webview_pane.bridge.send_notification("ui/notifications/tool-input", json!({
    "arguments": { "action": action, "args": args }
}));
```

This is the "protocol boundary convergence" — it's a single match arm once the bridge infrastructure exists.

### Full Protocol Message Table

| Method | Direction | Plexi responsibility |
|---|---|---|
| `ui/initialize` | App → Host | Respond with hostInfo, theme, container dims, CSP grants |
| `ui/notifications/initialized` | App → Host | Enable bidirectional; mark `pane.initialized = true` |
| `tools/call` | App → Host | Proxy to MCP server via `mcp_client`; return result |
| `tools/list` | App → Host | Return tools from MCP server filtered by visibility |
| `ui/message` | App → Host | Surface as Plexi notification or into linked terminal |
| `ui/request-display-mode` | App → Host | Resize/reposition WebView; update pane chrome |
| `ui/open-link` | App → Host | `open::that(url)` — system browser |
| `ui/update-model-context` | App → Host | Future: inject into context sidebar |
| `ui/notifications/tool-input` | Host → App | Fire when linked tool is invoked or `app action` dispatched |
| `ui/notifications/tool-result` | Host → App | Push tool result to app |
| `ui/notifications/host-context-changed` | Host → App | Theme change, resize, display mode change |
| `ui/notifications/size-changed` | Host → App | Sent after WebView bounds update |
| `ui/resource-teardown` | Host → App | Sent before pane close |

### MCP Client (Direction A Only)

This is a significant new piece. A WebView pane is a *client* of an MCP server (it calls tools *on* the server). Plexi currently only has an MCP *server* (`mcp_server.rs`). Direction A requires a minimal MCP client:

- HTTP client that connects to an external MCP server URL
- Maintains `initialize`/`initialized` session lifecycle
- Proxies `tools/call` requests from the WebView bridge to the server
- Returns results back into the bridge

Scope: ~400–600 lines. Could use `reqwest` (already in deps?) or the raw TCP approach matching `mcp_server.rs`.

### Workspace Save/Restore

`SavedPaneKind` (`src/workspace/mod.rs:61`) currently only knows `Terminal | App | Portal`. WebView panes need:

```rust
pub enum SavedPaneKind {
    Terminal,
    App,
    Portal { context_id: u64 },
    WebView { mcp_server_url: String, resource_uri: String },  // new
}
```

Plus restore logic in `src/app/mod.rs:769` to re-create the WebView and reconnect the MCP client.

---

## Unknown Unknowns — What Will Bite a Junior Dev

These are not obvious from reading the code or spec. They are ordered by likelihood of causing a multi-day block.

**1. wry + eframe window handle timing.**
wry needs a valid `HasWindowHandle` before the first frame renders. eframe doesn't expose the raw window handle until after the GPU surface is initialized. The safe place to create wry WebViews is inside a `setup` callback or gated on a "first frame received" flag. Attempting to create before the window is ready panics.

**2. NSScreen scale factor vs egui pixels_per_point.**
These are not always equal. A detached external monitor at a different scale factor can produce a mismatch. Always use `NSScreen.backingScaleFactor` via `objc2` for WKWebView positioning. Never use egui's `pixels_per_point` for this.

**3. WebView draws over all overlays.**
WKWebView is a native NSView and ignores the Metal surface's draw order. It will render over Plexi's command palette, modals, and any overlay. You must call `webview.set_visible(false)` whenever `host_state.modal_open` is true (or any overlay is active). This requires piping overlay state out of the render path and into the WebView manager — currently that state is local to the egui paint pass.

**4. The SDK uses `window.parent.postMessage`, not `window.webkit.messageHandlers`.**
`@modelcontextprotocol/ext-apps` is designed for iframe-in-webpage transport. Inside WKWebView there is no parent frame. Without the bridge shim (see above), `ui/initialize` never fires and the app hangs silently on startup. This is the single most common integration failure for native MCPUI hosts.

**5. `tools/call` visibility enforcement.**
The spec says app-only tools (visibility: `["app"]`) must not be forwarded to the LLM's tool list, and the host must reject external invocations of them. The MCP client proxy layer must filter `tools/list` responses before returning them to the WebView, and reject `tools/call` targeting app-only tools from anything other than the WebView itself.

**6. `ui/initialize` requires `toolInfo` context.**
The host must pack the triggering tool's name and arguments into `hostContext.toolInfo` in the `ui/initialize` response. This means pane creation must carry `{ triggering_tool_name, triggering_tool_args }` so they're available when the WebView fires `ui/initialize`. A WebView pane opened without a triggering tool (e.g. from `plexi app open`) sends `toolInfo: null`.

**7. Local HTML loading origin semantics.**
Using `loadFileURL` for local HTML gives the WebView a `file://` origin. CSP rules, `fetch()`, and `localStorage` all behave differently at `file://` than at `https://`. Use `loadHTMLString:baseURL:` with a synthetic `https://plexi-app-sandbox/<pane_id>.app` base URL instead. This gives consistent web-origin semantics without requiring a local HTTP server.

**8. Direction A requires an MCP client, not just a server.**
The existing `mcp_server.rs` is an MCP server (external clients call into it). A WebView pane is an MCP *client* (it calls out to a server). These are opposite roles. Don't try to repurpose `mcp_server.rs` for this — write a separate `mcp_client.rs`.

**9. Permissions modal for camera/mic/geolocation/clipboard.**
The spec allows apps to request these capabilities via `_meta.ui.permissions`. WKWebView will surface its own OS permission dialog if you don't intercept it. Intercept via `WKUIDelegate` and route through Plexi's existing notification/modal system so the user sees a Plexi-branded consent dialog, not a raw browser prompt. Silently granting or silently blocking are both wrong.

---

## Lab Note: MCPUI Inside a Terminal (PTY)

This was raised as a curiosity question. It is genuinely interesting and technically feasible because Plexi owns both ends: the PTY and the terminal renderer (`egui_term`).

**The idea:** A process running inside a Plexi terminal outputs a special OSC escape sequence declaring an MCPUI region:
```
OSC 9999 ; mcpui ; <resource_uri> ; <width_cols> ; <height_rows> ST
```

The Plexi terminal renderer (`egui_term`) intercepts this sequence, reserves a rectangular grid region at the current cursor position, and renders a WKWebView (or texture) into that region — exactly how iTerm2/Kitty render inline images.

**Why Plexi can do this when other terminals can't:**
Most terminals render character cells to pixels and have no path from "cell region" to "native view". Plexi's renderer is egui-based and already knows pixel positions for every cell. Mapping a cell rect to a WKWebView overlay is the same positioning math used for a full WebView pane — just constrained to the terminal grid rect.

**The mouse problem:** When the user clicks inside the reserved region, the PTY normally receives mouse tracking sequences. Instead, Plexi detects the click is within an active MCPUI region, suppresses the PTY mouse event, and routes the pointer event into the WebView bridge as a `ui/` interaction.

**What this enables:** A tool running in a terminal (e.g. a Python script, a CLI agent) emits a single escape sequence and gets a full web UI rendered inline in the terminal pane, with bidirectional communication back through the MCP protocol. No new pane type, no split. The HTML app lives inside the terminal.

**Precedents:** This is the same mechanism as:
- iTerm2 inline images (OSC 1337)
- Kitty graphics protocol (APC sequences)
- Wezterm's image rendering
- DomTerm's HTML embedding in terminal cells

**The significant difference from those:** MCPUI is interactive and bidirectional. Images are static. A fully interactive WebView widget inside a terminal cell region — with tool call proxying back to the process that spawned it — would be novel.

**Feasibility verdict:** Technically sound. The hard parts (WKWebView overlay, postMessage bridge, mouse routing) are the same as Direction A. This is Direction A + "anchor to a terminal cell rect instead of a pane rect." If Direction A ships, terminal embedding is a relatively small delta.

**Open questions for this variant:**
- What happens when the terminal scrolls? The MCPUI region is anchored to a grid position — does it scroll with the content, or stay fixed?
- How does the process regain control of the cell region (tear it down) — a second OSC sequence?
- Can multiple MCPUI regions coexist in one terminal?

Worth filing as a separate idea issue once Direction A is scoped.

---

## Open Questions Before Work Starts

1. **Direction priority.** Ship B (serve `ui://` from existing HTTP bridge) first? Confirmed yes — B is the right first step.

2. **Direction A: which MCP servers?** When a WebView pane opens, it connects to the MCP server that exposed the `ui://` resource. Does Plexi support arbitrary remote URLs (cloud), or only localhost? Scope affects the MCP client design significantly.

3. **Manifest design.** New `type = "mcpui"` (separate from PGAP) vs existing `type = "app"` + optional `[app.mcpui]` section (PGAP app that also ships HTML). Lean toward a separate type to keep the two runtimes' manifests clean.

4. **`app action` for WebView panes.** Should `plexi app action <webview_pane_id> <action>` dispatch `ui/notifications/tool-input`? Or is `app action` PGAP-only and WebView panes get a separate dispatch verb?

5. **Inline terminal MCPUI.** File as a separate idea issue once Direction A is scoped.

---

## Files That Change (Summary)

| File | Direction | What changes |
|---|---|---|
| `src/app/registry.rs` | B | Add `McpUiSection`, `McpUiCsp` structs; add `mcpui` field to `AppManifestApp` |
| `src/process_app/mcp_server.rs` | B | Add `resources/list`, `resources/read` handlers; advertise extension in `initialize` |
| `src/host/pane.rs` | A | Add `Pane::WebView(Box<WebViewPane>)` variant |
| `src/pane_ops/create.rs` | A | WebView pane creation: wry init, bridge setup, MCP client connection |
| `src/app/lifecycle.rs` | A | `SendAppAction` match arm for WebView panes |
| `src/process_app/mcp_client.rs` | A | New file: minimal MCP HTTP client for WebView→server proxying |
| `src/workspace/mod.rs` | A | `SavedPaneKind::WebView { mcp_server_url, resource_uri }` |
| `src/app/mod.rs` | A | WebView pane restore logic |
| `src/spatial/tiling.rs` | A | `PaneKind::WebView` variant for tile layout |
| `Cargo.toml` | A | Add `wry` under `[target.'cfg(target_os = "macos")'.dependencies]` |

Do not touch: PGAP stdio path, egui render pipeline, `ProcessApp`, `AppRuntime`, `TerminalPane`.
