Always confirm best practices by researching the docs.

## Testing

- **Primary e2e tests:** `just test-binary-run` — runs WebDriverIO tests against the real Tauri binary with a real PTY. Build first with `just test-binary` if needed.
- **Rust unit tests:** `just test-tauri`
- **Playwright tests are deprecated** — they use a mock session bridge and can't test real terminal behavior. Prefer binary e2e tests for all iteration.
- On Linux without a graphical session, binary tests need: `WAYLAND_DISPLAY=wayland-1 just test-binary-run`
