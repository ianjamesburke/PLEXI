# plexi.json Manifest Specification

**Version:** 1.0  
**Schema:** `schemas/plexi-manifest-schema.json`

---

## Overview

`plexi.json` is a declarative app format for Plexi. It lets you define a Plexi app without writing code — useful for dashboards, info panels, status displays, and read-only data views where a static or lightly-dynamic UI is all you need.

It serves two distinct purposes:

**1. Local app definition**  
Drop a `plexi.json` in any directory (or at `.plexi/plexi.json` for a project-scoped app). Plexi discovers it and renders the declared UI without requiring a compiled binary or Python script. For static use cases this replaces `manifest.toml` + a draw-protocol subprocess entirely.

**2. Website discovery via `/.well-known/plexi.json`**  
Any website can serve a manifest at `/.well-known/plexi.json` (per RFC 8615). When a user navigates to a URL in Plexi or pastes one into the command palette, Plexi fetches this path. If a valid manifest is returned, Plexi renders a native panel for the site instead of (or alongside) a browser pane. Wikipedia could show a clean article reader; a SaaS API could expose a live dashboard — all without a browser.

---

## Format

All fields at the top level of the JSON object. Unknown fields are ignored for forward compatibility.

### Required Fields

| Field     | Type   | Description                                    |
|-----------|--------|------------------------------------------------|
| `name`    | string | Display name shown in the Plexi title bar and app launcher |
| `version` | string | Manifest version in semver format (`"1.0.0"`)  |

### Optional Fields

| Field        | Type            | Description |
|--------------|-----------------|-------------|
| `description`| string          | Short description shown in the app launcher |
| `icon`       | string (URI)    | Path or URL to a PNG/SVG icon. Relative paths are resolved from the manifest's location |
| `display`    | enum (see below)| How the app window is presented. Default: `"standalone"` |
| `permissions`| string[]        | Capabilities the app requests. Declared here, granted by the user (see Permissions) |
| `endpoint`   | string (URI)    | WebSocket URL for dynamic apps. If absent, the app is static (see Static Mode) |
| `draw`       | DrawCommand[]   | Static frame to render when the app has no `endpoint`, or as a loading screen while the WebSocket connects |
| `min_width`  | number          | Minimum pane width in logical pixels |
| `min_height` | number          | Minimum pane height in logical pixels |
| `theme`      | object          | Color overrides for this app's surface (see Theme) |
| `author`     | string          | Author name or org |
| `homepage`   | string (URI)    | URL to the app's homepage or docs |

### `display` Values

Borrowed from the PWA manifest spec. Controls how Plexi allocates space for the app.

| Value          | Behavior |
|----------------|----------|
| `"standalone"` | Full pane — the app owns its entire allocated pane. Default. |
| `"panel"`      | Side panel — rendered in a fixed-width column alongside the terminal |
| `"overlay"`    | Floating overlay — rendered above the current pane, dismissible with Escape |

### `theme` Object

| Field        | Type   | Description |
|--------------|--------|-------------|
| `background` | string | Background color (CSS hex: `"#1e1e2e"`) |
| `foreground` | string | Primary text color |
| `accent`     | string | Accent/highlight color for selected items, links |

### `draw` Array — DrawCommand Objects

Each object in the `draw` array is a draw command. These are the same commands used by the live draw protocol, so a static manifest and a dynamic app speak the same language. Commands are rendered in order (painter's model — later commands draw on top).

| Command type    | Required fields                           | Optional fields                      |
|-----------------|-------------------------------------------|--------------------------------------|
| `"rect"`        | `x, y, w, h, fill` (string: CSS hex)     | `radius` (number, default `0`)       |
| `"text"`        | `x, y, text` (string), `size` (number), `color` (string) | `bold` (bool), `monospace` (bool) |
| `"line"`        | `x1, y1, x2, y2, color` (string)         | `width` (number, default `1`)        |
| `"list"`        | `items` (array of item objects)           | `selected` (int), `item_height` (number) |
| `"frame_done"`  | — (marks end of frame; optional in static mode) | — |

**List item object fields:** `label` (string, required), `secondary` (string, optional), `is_dir` (bool, optional).

All coordinates are in logical pixels relative to the top-left of the app surface. Plexi scales for HiDPI automatically.

---

## Permissions

Apps declare what they need. Users grant what they allow.

Permission strings follow a `domain[.access]` pattern:

| Permission          | What it allows |
|---------------------|----------------|
| `filesystem.read`   | Read files within the app's launch directory and subdirectories |
| `filesystem.write`  | Write files within the app's launch directory and subdirectories |
| `network`           | Make outbound HTTP/WebSocket requests |
| `terminal`          | Write commands to the linked terminal pane |
| `secrets`           | Request named secrets from the Plexi Keychain via the `SecretGet` API |

A permission declared in `plexi.json` is a request — it must be approved by the user before Plexi grants it. Plexi shows a one-time prompt on first launch. For `/.well-known/` discovery, the same approval flow runs on first connection to the site.

**Principle of least privilege:** Only declare permissions you actually use. Over-broad permission requests lower user trust and increase the chance of a manual downgrade.

---

## Transport

### Local File (Static or Subprocess)

When `plexi.json` is loaded from disk:

- If `endpoint` is absent: Plexi renders the `draw` array as a static frame. No subprocess is spawned.
- If `endpoint` is present and begins with `ws://` or `wss://`: Plexi connects to that WebSocket URL for dynamic rendering (see below).
- For local apps that wrap a subprocess, use `manifest.toml` instead — `plexi.json` is not designed for subprocess management.

### WebSocket Remote (Dynamic)

When `endpoint` is set to a WebSocket URI, Plexi acts as a WebSocket client:

1. Plexi connects to `endpoint`.
2. On connection, Plexi sends an `init` event: `{"type": "init", "width": N, "height": N, "capabilities": [...]}`.
3. The server sends draw commands in response to `render` events (same JSON format as the static `draw` array).
4. The session continues with the same event/draw-command protocol used by local out-of-process apps:
   - **Plexi → Server:** `init`, `render`, `resize`, `key`, `click`, `command`, `shutdown`
   - **Server → Plexi:** `rect`, `text`, `line`, `list`, `run_in_terminal`, `cd`, `frame_done`

The `draw` field (if present) is used as a loading/splash frame while the WebSocket connection is being established.

**Security:** WebSocket apps discovered via `/.well-known/` run in the `sandboxed` trust level. Capabilities declared in the manifest require explicit user approval before use. Apps can be elevated to `trusted` via the Permissions Manager.

---

## Discovery

### How Plexi Discovers `/.well-known/plexi.json`

When a user navigates to a URL in Plexi (via the command palette or a terminal link), Plexi sends a HEAD request to `/.well-known/plexi.json` on that origin with the header:

```
X-Plexi-Client: 1
```

If the response is `200 OK` with `Content-Type: application/json`, Plexi fetches the full manifest and proceeds to render. If the path returns `404` or the response is not valid JSON matching the schema, Plexi falls back to normal behavior (opening in a browser pane or ignoring the link).

### Server-Side Requirement

To enable discovery, a site must:

1. Serve a valid `plexi.json` at `/.well-known/plexi.json`.
2. Return `Content-Type: application/json`.
3. Include `Access-Control-Allow-Origin: *` (or the appropriate CORS policy) so Plexi's fetch is not blocked.

RFC 8615 establishes `/.well-known/` as the standard namespace for well-known URIs. No IANA registration is required for `plexi.json` in private/experimental use.

### Privacy Consideration

Plexi only probes `/.well-known/plexi.json` on explicit user navigation — not on every terminal URL or passive link hover. The `X-Plexi-Client: 1` header reveals that the request originated from Plexi; sites can use this to serve different content to Plexi clients vs. browsers.

---

## Static Mode

A manifest with no `endpoint` key is a **static app**. Plexi renders the `draw` array once and displays it. No subprocess is spawned, no network connection is made.

Static mode is suitable for:

- Splash screens and loading states
- Read-only info panels and dashboards built from pre-baked data
- Placeholder UIs while a dynamic endpoint is in development
- Website discovery manifests that show a branded welcome screen before the WebSocket connects

In static mode, scroll is the only interactivity. If a `list` command is included, the user can scroll through it with keyboard or mouse. Key/click events are not forwarded to the app (there is no app to forward them to).

To add interactivity, add an `endpoint`.

---

## Examples

### Minimal Example

The smallest valid `plexi.json` — three fields, static mode, no draw commands.

```json
{
  "name": "My Dashboard",
  "version": "1.0.0"
}
```

Renders an empty surface with default theme colors.

---

### Full Example

All fields in use. Shows a static splash screen while the WebSocket connects.

```json
{
  "name": "Deploy Dashboard",
  "version": "1.2.0",
  "description": "Live deployment status across all services",
  "icon": "assets/icon.png",
  "display": "standalone",
  "permissions": ["network", "secrets"],
  "endpoint": "wss://dashboard.example.com/plexi",
  "min_width": 600,
  "min_height": 400,
  "author": "Example Corp",
  "homepage": "https://dashboard.example.com/docs",
  "theme": {
    "background": "#0d1117",
    "foreground": "#e6edf3",
    "accent": "#58a6ff"
  },
  "draw": [
    { "type": "rect", "x": 0, "y": 0, "w": 800, "h": 600, "fill": "#0d1117" },
    { "type": "text", "x": 40, "y": 40, "text": "Deploy Dashboard", "size": 24, "color": "#e6edf3", "bold": true },
    { "type": "text", "x": 40, "y": 80, "text": "Connecting...", "size": 14, "color": "#8b949e" }
  ]
}
```

---

### Wikipedia Example

What `en.wikipedia.org` could serve at `/.well-known/plexi.json`. The `draw` field renders a branded splash screen while the WebSocket connects. The `endpoint` is hypothetical.

```json
{
  "name": "Wikipedia",
  "version": "1.0.0",
  "description": "The free encyclopedia",
  "icon": "https://en.wikipedia.org/static/favicon/wikipedia.ico",
  "display": "standalone",
  "permissions": ["network"],
  "endpoint": "wss://en.wikipedia.org/.well-known/plexi-ws",
  "min_width": 500,
  "min_height": 300,
  "author": "Wikimedia Foundation",
  "homepage": "https://www.wikipedia.org",
  "theme": {
    "background": "#202122",
    "foreground": "#eaecf0",
    "accent": "#3366cc"
  },
  "draw": [
    { "type": "rect",  "x": 0,   "y": 0,   "w": 800, "h": 600, "fill": "#202122" },
    { "type": "text",  "x": 40,  "y": 40,  "text": "Wikipedia", "size": 28, "color": "#eaecf0", "bold": true },
    { "type": "text",  "x": 40,  "y": 82,  "text": "The Free Encyclopedia", "size": 14, "color": "#9ea3b0" },
    { "type": "line",  "x1": 40, "y1": 110, "x2": 400, "y2": 110, "color": "#3d3d3d" },
    { "type": "text",  "x": 40,  "y": 130, "text": "Connecting to article reader...", "size": 13, "color": "#6e7681" }
  ]
}
```

When the WebSocket connects, the server takes over and renders a full search+article reader using the live draw protocol — the same UI provided by the existing `examples/wikipedia/wikipedia.py` app, but served remotely without any local files.
