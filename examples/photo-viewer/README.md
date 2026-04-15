# photo-viewer

Plexi example: external single-image viewer written in Rust.

Receives exactly one file path as `argv[1]`, decodes the image, and renders a
zoomable/pannable view. This app is intentionally **single-image only** — no
directory awareness, no next/prev, no file enumeration, no thumbnail strip. If
you want a library experience, compose this viewer with the file browser via
`spawn_app` (landing in a follow-up PR). Until then the viewer is standalone.

## Keybindings

| Action | Keys |
|---|---|
| Pan | `h` `j` `k` `l` or arrows |
| Zoom in / out | `+` / `-` (cursor-anchored on scroll) |
| Reset view | `r` or `0` |
| Fullscreen toggle | `f` or `Cmd+Enter` (host) |
| Save edited copy | `Cmd+Shift+S` (v1 stub) |

## Build & install

```sh
cd examples/photo-viewer
cargo build --release
# produces: target/release/photo-viewer

# install into the active Plexi build (alpha shown here)
mkdir -p ~/.plexi-alpha/apps/photo-viewer
cp -r . ~/.plexi-alpha/apps/photo-viewer/
chmod +x ~/.plexi-alpha/apps/photo-viewer/target/release/photo-viewer
```

The `manifest.toml` points `entry` at `target/release/photo-viewer` relative
to the installed app directory, so the `target/` directory must be copied
alongside `manifest.toml`.

## Run standalone

```sh
target/release/photo-viewer ~/Pictures/sunset.jpg
```

The app speaks Plexi's JSON-lines protocol on stdin/stdout. Run it under the
Plexi host to see the UI — invoking it in a normal terminal will block waiting
for `init`/`render` events.

## Tests

```sh
cargo test
```

Unit tests cover the fit-to-window, zoom-at-cursor, and reset math. One
integration test spawns the binary against a 4×4 red PNG fixture and verifies
that `init` + `render` produces `rect` draw commands followed by `frame_done`.
