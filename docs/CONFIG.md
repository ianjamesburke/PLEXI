# Plexi Configuration Reference

Full config reference for `~/.plexi/config.toml` (or channel equivalent: `~/.plexi-alpha/`, `~/.plexi-beta/`, `~/.plexi-pr-N/`).

Edit: `plexi config edit` | Check: `plexi config check` | Reset: `plexi config reset`

Online: https://plexiapp.com/docs/config

---

## Top-level keys

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `config_version` | integer | 1 | Config format version. |
| `font_size` | float | 14.0 | Terminal font size in points. |
| `pane_gap` | float | 4.0 | Inter-pane gap in pixels. Range: 0–20. |
| `pane_title_font_size` | float | 11.0 | Pane title bar font size. Range: 6–32. |
| `confirm_quit` | bool | true | Require triple Cmd+Q to exit. |
| `confirm_close` | bool | false | Show dialog before Cmd+W closes a pane. |
| `focus_history_depth` | integer | 100 | Pane focus history depth (Cmd+[ / Cmd+]). |
| `osc_pane_title` | bool | true | Apply OSC 0/1/2 title sequences from running processes as pane names. |

---

## [theme]

Apply a named preset and optionally override individual color tokens.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `preset` | string | — | Theme preset name. See options below. |
| `accent` | hex color | preset | Accent color (highlights, badges). |
| `bg_darkest` | hex color | preset | Darkest background layer. |
| `bg_sidebar` | hex color | preset | Sidebar background. |
| `bg_toolbar` | hex color | preset | Toolbar background. |
| `terminal_bg` | hex color | preset | Terminal pane background. |
| `bg_hover` | hex color | preset | Row/item hover background. |
| `bg_sidebar_hover` | hex color | preset | Sidebar item hover background. |
| `bg_active` | hex color | preset | Active/selected item background. |
| `text_primary` | hex color | preset | Primary text color. |
| `text_dim` | hex color | preset | Dimmed/secondary text. |
| `text_section` | hex color | preset | Section header text. |
| `border` | hex color | preset | Border/divider color. |
| `foreground` | hex color | preset | Terminal default foreground. |
| `background` | hex color | preset | Terminal default background. |
| `black` | hex color | preset | ANSI color 0. |
| `red` | hex color | preset | ANSI color 1. |
| `green` | hex color | preset | ANSI color 2. |
| `yellow` | hex color | preset | ANSI color 3. |
| `blue` | hex color | preset | ANSI color 4. |
| `magenta` | hex color | preset | ANSI color 5. |
| `cyan` | hex color | preset | ANSI color 6. |
| `white` | hex color | preset | ANSI color 7. |
| `bright_black` | hex color | preset | ANSI color 8. |
| `bright_red` | hex color | preset | ANSI color 9. |
| `bright_green` | hex color | preset | ANSI color 10. |
| `bright_yellow` | hex color | preset | ANSI color 11. |
| `bright_blue` | hex color | preset | ANSI color 12. |
| `bright_magenta` | hex color | preset | ANSI color 13. |
| `bright_cyan` | hex color | preset | ANSI color 14. |
| `bright_white` | hex color | preset | ANSI color 15. |
| `bright_foreground` | hex color | preset | Bright terminal foreground. |

**Available presets:** `catppuccin-mocha`, `catppuccin-latte`, `dracula`, `tokyo-night`, `gruvbox-dark`, `nord`, `solarized-dark`, `solarized-light`

Example:
```toml
[theme]
preset = "catppuccin-mocha"
accent = "#cba6f7"   # override one token
```

---

## [notifications]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | true | Master switch. If false, all notifications are silently dropped at arrival. |
| `focus_mode` | bool | false | When true, no notification auto-surfaces regardless of priority. Everything queues silently; open Cmd+Shift+A to review. |
| `interrupt_threshold` | integer | 100 | Minimum priority that auto-opens the modal. Below this value, notifications queue silently. At or above it, arrival auto-opens the modal. |

Priority tiers:
- `0` = LOW (background info)
- `50` = NORMAL (standard confirmations)
- `100` = HIGH (needs attention soon) — default threshold
- `200` = CRITICAL (interrupt-level)

---

## [ai]

AI broker configuration for apps using the `ai.query` capability.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `backend` | string | `"openrouter"` | Provider: `"openrouter"` or `"ollama"`. |
| `per_app_daily_usd` | float | 1.0 | Per-app daily spend cap in USD. |
| `global_daily_usd` | float | 10.0 | Global daily spend cap across all apps in USD. |

---

## [ai.openrouter]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `api_key_env` | string | `"OPENROUTER_API_KEY"` | Environment variable holding the API key. Never paste keys here — export in your shell profile. |
| `model_low` | string | `"qwen/qwen3.6-flash"` | Fast/cheap model tier. |
| `model_medium` | string | `"xiaomi/mimo-v2.5-pro"` | Standard model tier. |
| `model_high` | string | `"anthropic/claude-fable-5"` | High-capability model tier. |

---

## [ai.ollama]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `host` | string | `"http://localhost:11434"` | Ollama server URL. |
| `model_low` | string | — | Fast/cheap model tier. |
| `model_medium` | string | — | Standard model tier. |
| `model_high` | string | — | High-capability model tier. |

---

## [log]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `level` | string | `"info"` | Log level: `error`, `warn`, `info`, `debug`. |
| `retention_days` | integer | 30 | Days to keep dated log archives. |

Log file locations:
- Main: `~/.plexi/plexi.log`
- Alpha: `~/.plexi-alpha/plexi.log`
- Beta: `~/.plexi-beta/plexi.log`

---

## [keybindings]

Override any default keybinding. Format: `"modifier+key"` (case-insensitive).

Modifiers: `cmd`/`command`, `shift`, `ctrl`/`control`, `alt`/`opt`/`option`.

Keys: `a`–`z`, `0`–`9`, `enter`, `escape`, `tab`, `space`, `backspace`, `delete`, `up`, `down`, `left`, `right`, `open_bracket`, `close_bracket`, `backslash`, `slash`, `comma`, `period`, `equals`, `minus` (and symbol aliases: `[`, `]`, `\`, etc.).

| Action | Description |
|--------|-------------|
| `quit` | Quit Plexi. |
| `close_pane` | Close the active pane. |
| `toggle_command_palette` | Open/close the command palette. |
| `split_horizontal` | Split active pane horizontally. |
| `split_vertical` | Split active pane vertically. |
| `split_right` | Split and open new pane to the right. |
| `split_down` | Split and open new pane below. |
| `swap_pane_left` | Swap active pane with the one to the left. |
| `swap_pane_down` | Swap active pane with the one below. |
| `swap_pane_up` | Swap active pane with the one above. |
| `swap_pane_right` | Swap active pane with the one to the right. |
| `send_pane_left` | Move active pane to the left. |
| `send_pane_down` | Move active pane downward. |
| `send_pane_up` | Move active pane upward. |
| `send_pane_right` | Move active pane to the right. |
| `navigate_left` | Focus pane to the left. |
| `navigate_down` | Focus pane below. |
| `navigate_up` | Focus pane above. |
| `navigate_right` | Focus pane to the right. |
| `new_tab` | Open a new tab. |
| `next_tab` | Switch to next tab. |
| `prev_tab` | Switch to previous tab. |
| `first_tab` | Switch to first tab. |
| `last_tab` | Switch to last tab. |
| `nav_back` | Navigate focus history backward. |
| `focus_history_forward` | Navigate focus history forward. |
| `toggle_sidebar` | Show/hide the sidebar. |
| `toggle_zoom` | Zoom/unzoom the active pane. |
| `toggle_shortcuts` | Show/hide keyboard shortcut overlay. |
| `rename_context` | Rename the active context. |
| `rename_pane` | Rename the active pane. |
| `new_context` | Create a new context. |
| `new_page_right` | Open new page to the right. |
| `toggle_minimap` | Show/hide the minimap. |
| `scroll_up` | Scroll active pane up. |
| `scroll_down` | Scroll active pane down. |
| `increase_font_size` | Increase terminal font size. |
| `decrease_font_size` | Decrease terminal font size. |
| `open_file_browser` | Open the file browser. |
| `open_quick_note` | Open quick note entry. |
| `open_config` | Open config file in editor. |
| `reload_config` | Reload config without restart. |
| `open_secrets_manager` | Open the secrets manager. |
| `force_reload_app` | Force-reload the active app. |
| `toggle_notification_modal` | Open/close the notification modal. |
| `open_scratchpad` | Open the scratchpad. |
| `set_context_root_from_cwd` | Set the context root to the CWD of the active pane. |
| `push_to_subcontext` | Push the active pane into a subcontext. |
| `new_child_context` | Create a child context under the current one. |
| `hide_pane` | Hide the active pane. |
| `park_context` | Park (minimize) the current context. |
| `open_notes_picker` | Open the notes picker overlay. |

Example:
```toml
[keybindings]
toggle_command_palette = "cmd+p"
open_config            = "cmd+comma"
```

---

## [agents]

Command templates used when Plexi skills dispatch coding agent panes. `{cmd}` is replaced with the prompt or slash command at dispatch time.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `low` | string | `claude --model claude-haiku-4-5 --dangerously-skip-permissions '{cmd}'` | Fast/cheap tasks. |
| `medium` | string | `claude --model claude-sonnet-4-6 --dangerously-skip-permissions '{cmd}'` | Standard work. |
| `high` | string | `claude --dangerously-skip-permissions '{cmd}'` | Complex/autonomous tasks. |

---

## [cli]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `tips` | bool | true | Print contextual tips after CLI commands. Set to `false` to suppress. |

---

## [beta]

Experimental features. Flip any flag to `true` and restart to enable.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `crt` | bool | false | Retro CRT scanlines + green phosphor tint. |
| `ghost` | bool | false | Unfocused panes render at reduced opacity. |
| `ghost_opacity` | float | 0.9 | Opacity for unfocused panes. Range: 0.0–1.0. Setting this implies `ghost = true`. |

---

## Workspace overrides

Project-level config at `.plexi/workspace.toml` (main channel) or `.plexi-alpha/workspace.toml` etc. for channel builds. Any key set here overrides the global config for that project. Fields not present in the workspace config inherit from the global config.

Initialize with: `plexi workspace init`
