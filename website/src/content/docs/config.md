---
title: Configuration
description: Edit config.toml, choose a theme, wire AI models, and override shortcuts.
order: 2
---

Plexi reads `config.toml` from the active channel profile. Alpha uses `~/.plexi-alpha/config.toml`, beta uses `~/.plexi-beta/config.toml`, main uses `~/.plexi/config.toml`, and PR builds use `~/.plexi-pr-<N>/config.toml`.

Changes take effect on the next launch unless a command says otherwise.

## Channel discipline

Alpha config stays default. `just install` refreshes `~/.plexi-alpha/config.toml` from the built-in template, and PR builds seed from alpha. Do not use alpha for personal overrides you expect to keep.

Beta config is the staging ground. `~/.plexi-beta/config.toml` is not reset by alpha installs, so it is the right place to test migrations, advanced settings, and personal config before promoting a release.

PR builds are isolated. A PR build reads `~/.plexi-pr-<N>/config.toml`; use the `plexi-pr-<N>` binary when checking a PR-specific config behavior.

## Commands

```sh
plexi config edit
plexi config check
plexi config reset
```

`config edit` opens the active channel config. `config check` validates known keys and TOML syntax. `config reset` backs up the current file to `config.toml.bak`, then writes the built-in default template.

## Reference

### Top-level (`config.toml`)

| Field | Type | Default | Description |
|---|---|---|---|
| `config_version` | integer | — | — |
| `font_size` | float | — | — |
| `pane_gap` | float | 4.0 | Inter-pane gap width in pixels. Default: 4.0. Clamped to [0.0, 20.0]. |
| `pane_title_font_size` | float | 11.0 | Pane title bar font size. Default: 11.0. Clamped to [6.0, 32.0]. |
| `osc_pane_title` | bool | true | Apply OSC 0/1/2 title sequences from terminal processes as pane names. Defaults to true; set false to keep terminal pane names manual-only. |
| `theme_preset` | string | — | — |
| `confirm_quit` | bool | true | Set to false to quit immediately on Cmd+Q without triple-press confirmation (default: true). |
| `confirm_close` | bool | true | Set to false to close panes immediately on Cmd+W without a confirmation dialog (default: true). |
| `confirm_context_close` | bool | true | Set to false to close contexts immediately on Cmd+Shift+W without a confirmation dialog (default: true). |
| `focus_history_depth` | integer | — | — |

### Theme (`[theme]`)

Pick a preset and optionally override individual colors. Individual color fields override preset values when both are set.

Available presets: `catppuccin-mocha`, `catppuccin-latte`, `dracula`, `tokyo-night`, `tokyo-day`, `gruvbox-dark`, `gruvbox-light`, `nord`, `solarized-dark`, `solarized-light`, `matrix`, `bios`, `plexi-night`, `plexi-day`.

| Field | Type | Default | Description |
|---|---|---|---|
| `preset` | string | — | Theme preset name. Applied first; individual color fields override the preset. |
| `bg_darkest` | string | — | — |
| `bg_sidebar` | string | — | — |
| `bg_toolbar` | string | — | — |
| `terminal_bg` | string | — | — |
| `bg_hover` | string | — | — |
| `bg_sidebar_hover` | string | — | — |
| `bg_active` | string | — | — |
| `text_primary` | string | — | — |
| `text_dim` | string | — | — |
| `text_section` | string | — | — |
| `accent` | string | — | — |
| `border` | string | — | — |
| `foreground` | string | — | — |
| `background` | string | — | — |
| `black` | string | — | — |
| `red` | string | — | — |
| `green` | string | — | — |
| `yellow` | string | — | — |
| `blue` | string | — | — |
| `magenta` | string | — | — |
| `cyan` | string | — | — |
| `white` | string | — | — |
| `bright_black` | string | — | — |
| `bright_red` | string | — | — |
| `bright_green` | string | — | — |
| `bright_yellow` | string | — | — |
| `bright_blue` | string | — | — |
| `bright_magenta` | string | — | — |
| `bright_cyan` | string | — | — |
| `bright_white` | string | — | — |
| `bright_foreground` | string | — | — |
| `pip_working` | string | — | — |
| `pip_idle` | string | — | — |
| `pip_blocked` | string | — | — |
| `pip_dim` | float | — | — |

### Effects (`[effects]`)

| Field | Type | Default | Description |
|---|---|---|---|
| `crt` | bool | — | — |
| `ghost` | bool | — | — |
| `ghost_opacity` | float | 0.75 | Opacity for unfocused panes when ghost is enabled. Range 0.0–1.0; default 0.75. Setting this implies ghost = true unless `ghost = false` is explicit. |

### Log (`[log]`)

`level` accepts `error`, `warn`, `info`, or `debug`. Applies live on config save — no restart required.

| Field | Type | Default | Description |
|---|---|---|---|
| `level` | string | — | — |
| `retention_days` | integer | — | — |

### Notifications (`[notifications]`)

Visible notifications open the modal unless `focus_mode` is enabled. There is one notification level.

| Field | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | true | Master switch. If false, incoming notifications are silently dropped — apps still send them, but the modal never appears and the queue stays empty. Defaults to true. |
| `focus_mode` | bool | false | Focus mode. When true, no notification auto-surfaces. Everything queues silently; the user reviews via Cmd+Shift+A. Defaults to false. |
| `sound` | string | — | Path to an audio file (WAV, MP3, FLAC, OGG) played once when a notification arrives, so an agent waiting on input can pull a human back while Plexi is in the background. Unset (the default) means no sound; an explicitly empty string also means no sound, so a workspace overlay can set `sound = ""` to silence a globally configured cue. The cue only fires for a notification that may interrupt: `enabled = false` drops it before the cue, `focus_mode = true` suppresses it, and an invisible notification arrives silently. |

### AI (`[ai]`)

API keys are NOT stored in `config.toml`. Export `OPENROUTER_API_KEY` in your shell profile (`~/.zshrc`, `~/.zprofile`, etc.), or store it in the Plexi secret store:

```sh
plexi secret set openrouter-api-key --global
```

| Field | Type | Default | Description |
|---|---|---|---|
| `backend` | string | — | Backend selection: `"openrouter"` (default), `"ollama"`, or `"local"`. |
| `per_app_daily_usd` | float | $1.00 | Per-app daily spend cap in USD. Default $1.00. |
| `global_daily_usd` | float | $10.00 | Global daily spend cap across all apps in USD. Default $10.00. |

#### OpenRouter (`[ai.openrouter]`)

| Field | Type | Default | Description |
|---|---|---|---|
| `api_key_env` | string | `OPENROUTER_API_KEY` | Environment variable name for the API key. Default: `OPENROUTER_API_KEY`. |
| `model_low` | string | — | Low-tier model. e.g. "qwen/qwen3.6-flash" |
| `model_medium` | string | — | Medium-tier model. e.g. "xiaomi/mimo-v2.5" |
| `model_high` | string | — | High-tier model. e.g. "anthropic/claude-fable-5" |

#### Ollama (`[ai.ollama]`)

| Field | Type | Default | Description |
|---|---|---|---|
| `host` | string | `http://localhost:11434` | Ollama host URL. Default: `http://localhost:11434`. |
| `model_low` | string | — | Low-tier model. e.g. "llama3.2:3b" |
| `model_medium` | string | — | Medium-tier model. e.g. "llama3.3:70b" |
| `model_high` | string | — | High-tier model. e.g. "qwq:32b" |

#### Local OpenAI-compatible (`[ai.local]`)

| Field | Type | Default | Description |
|---|---|---|---|
| `base_url` | string | — | Server base URL. Required — no default. e.g. "http://127.0.0.1:3456" |
| `api_key_env` | string | — | Environment variable name for the API key. Unset = no auth header (for local proxies that accept unauthenticated requests). |
| `model_low` | string | — | Low-tier model. e.g. "claude-haiku-4-5" |
| `model_medium` | string | — | Medium-tier model. e.g. "claude-opus-5" |
| `model_high` | string | — | High-tier model. e.g. "claude-fable-5" |

### Agents (`[agents]`)

Command templates for dispatching coding agents. `{cmd}` is replaced with the prompt or slash command at dispatch time.

| Field | Type | Default | Description |
|---|---|---|---|
| `low` | string | claude haiku with --dangerously-skip-permissions | Fast/cheap tasks. Default: claude haiku with --dangerously-skip-permissions. |
| `medium` | string | claude sonnet with --dangerously-skip-permissions | Standard work. Default: claude sonnet with --dangerously-skip-permissions. |
| `high` | string | claude with --dangerously-skip-permissions | Complex/autonomous tasks. Default: claude with --dangerously-skip-permissions. |

### Keybindings (`[keybindings]`)

Override only the shortcuts you want to change. Unknown or conflicting overrides log a warning at startup and keep the default binding.

Modifier keys: `cmd` (alias: `command`), `shift`, `ctrl` (alias: `control`), `alt` (aliases: `opt`, `option`).

Key names: `a`–`z`, `0`–`9`, `enter`, `escape`, `tab`, `space`, `backspace`, `delete`, `up`, `down`, `left`, `right`, `open_bracket`, `close_bracket`, `backslash`, `slash`, `comma`, `period`, `equals`, `minus`.

Each value is a string like `"cmd+p"` or `"cmd+shift+w"`.

| Action | Description |
|---|---|
| `quit` | quit |
| `close_pane` | close pane |
| `toggle_command_palette` | toggle command palette |
| `split_horizontal` | split horizontal |
| `split_vertical` | split vertical |
| `split_right` | split right |
| `split_down` | split down |
| `swap_pane_left` | swap pane left |
| `swap_pane_down` | swap pane down |
| `swap_pane_up` | swap pane up |
| `swap_pane_right` | swap pane right |
| `send_pane_left` | send pane left |
| `send_pane_down` | send pane down |
| `send_pane_up` | send pane up |
| `send_pane_right` | send pane right |
| `navigate_left` | navigate left |
| `navigate_down` | navigate down |
| `navigate_up` | navigate up |
| `navigate_right` | navigate right |
| `new_tab` | new tab |
| `next_tab` | next tab |
| `prev_tab` | prev tab |
| `next_context` | next context |
| `prev_context` | prev context |
| `move_context_up` | move context up |
| `move_context_down` | move context down |
| `nav_back` | nav back |
| `focus_history_forward` | focus history forward |
| `toggle_sidebar` | toggle sidebar |
| `toggle_zoom` | toggle zoom |
| `toggle_shortcuts` | toggle shortcuts |
| `rename_context` | rename context |
| `rename_pane` | rename pane |
| `new_context` | new context |
| `new_page_right` | new page right |
| `toggle_minimap` | toggle minimap |
| `scroll_up` | scroll up |
| `scroll_down` | scroll down |
| `increase_font_size` | increase font size |
| `decrease_font_size` | decrease font size |
| `open_file_browser` | open file browser |
| `open_quick_note` | open quick note |
| `open_config` | open config |
| `reload_config` | reload config |
| `open_secrets_manager` | open secrets manager |
| `open_assistant` | open assistant |
| `force_reload_app` | force reload app |
| `toggle_notification_modal` | toggle notification modal |
| `open_scratchpad` | open scratchpad |
| `push_to_subcontext` | push to subcontext |
| `new_child_context` | new child context |
| `set_context_root_from_cwd` | set context root from cwd |
| `hide_pane` | hide pane |
| `park_context` | park context |
| `open_notes_picker` | open notes picker |
| `close_context` | close context |

### CLI (`[cli]`)

| Field | Type | Default | Description |
|---|---|---|---|
| `tips` | bool | `true` | Print contextual tips after CLI commands. Default: `true`. Set to `false` to suppress. |

### File handlers (`[file_handlers]`)

Choose which app opens each file type from the File Explorer. Keys are bare lowercase extensions (e.g. `md`, not `.md`). Values are handler specs:

| Value | Behavior |
|---|---|
| `app:<id>` | Open in a Plexi app by its registry id (e.g. `app:text-editor`) |
| `os` | Hand to the OS default opener (`open` / `xdg-open`) |
| `cmd:<command>` | Reserved; falls back to `os` until wired |

A handler here overrides an app's own `file_types` association. Unmapped extensions fall through to manifest associations, then builtin media players, then the OS default.

### Marketplace (`[marketplace]`)

All fields are optional. Omitting the section uses the official `plexiapp.com` registry and CDN. Override only to point at a private registry or to test publishing flows.

| Field | Type | Default | Description |
|---|---|---|---|
| `registry_url` | string | official `plexiapp | Override the catalog index URL. Default: official `plexiapp.com` index. |
| `cdn_url` | string | official `plexiapp | Override the package CDN base. Default: official `plexiapp.com` CDN. |
| `submit_url` | string | — | Publisher submission endpoint. Unset = publishing prepared locally but not uploaded (fails closed with a clear message). |
| `account_backend` | string | — | Account/auth backend selector. `"plexi"` enables plexiapp.com accounts; unset / `"none"` = stub (login fails closed). Selects the provider in `crate::app::account::account_provider()`. |
| `account_url` | string | `crate::app::account::DEFAULT_ACCOUNT_URL` | Override the accounts service base URL. Default: `crate::app::account::DEFAULT_ACCOUNT_URL` (`plexiapp.com`). |
| `account_email` | string | — | Default email pre-filled by `plexi account login`. Unset = prompt. |

## Default config.toml

This is the built-in template Plexi writes when it creates or resets the active channel config.

```toml
# Plexi Configuration — full reference: https://plexiapp.com/docs/config | docs/CONFIG.md
# Edit: plexi config edit  |  Check: plexi config check  |  Reset: plexi config reset

config_version = 1

font_size = 14.0
# pane_gap = 4                 # inter-pane gap in pixels (0-20)
# pane_title_font_size = 12    # pane title bar font size (6-32)

confirm_quit  = true
confirm_close = false
confirm_context_close = true

# focus_history_depth = 100
osc_pane_title = true

[theme]
preset = "catppuccin-mocha"
# Presets: catppuccin-mocha, catppuccin-latte, dracula, tokyo-night, tokyo-day, gruvbox-dark, gruvbox-light, nord, solarized-dark, solarized-light, matrix, bios, plexi-night, plexi-day
# Uncomment to override individual colors:
# accent       = "#89b4fa"
# bg_darkest   = "#11111b"
# terminal_bg  = "#292a44"
# text_primary = "#cdd6f4"
# foreground   = "#e8e6ed"

[effects]
crt           = false
ghost         = true
ghost_opacity = 0.75

[notifications]
# enabled = true
# focus_mode = false
# sound = "/System/Library/Sounds/Ping.aiff"   # cue played on arrival; unset = silent

[ai]
backend = "openrouter"         # "openrouter" (cloud), "ollama", or "local" (OpenAI-compatible server)

[ai.openrouter]
api_key_env  = "OPENROUTER_API_KEY"
model_low    = "qwen/qwen3.6-flash"
model_medium = "xiaomi/mimo-v2.5"
model_high   = "anthropic/claude-fable-5"

# [ai.ollama]
# host         = "http://localhost:11434"
# model_low    = "llama3.2:3b"
# model_medium = "llama3.3:70b"
# model_high   = "qwq:32b"

# [ai.local]                   # any OpenAI-compatible server; api_key_env optional
# base_url     = "http://127.0.0.1:3456"
# model_low    = "claude-haiku-4-5"
# model_medium = "claude-opus-5"
# model_high   = "claude-fable-5"

# [log]
# level = "info"               # error | warn | info | debug — applies live on save, no restart
# retention_days = 30

# [keybindings]
# toggle_command_palette = "cmd+p"
# open_config            = "cmd+comma"
# close_context          = "cmd+shift+w"

[agents]
low    = "claude --model claude-haiku-4-5 --dangerously-skip-permissions '{cmd}'"
medium = "claude --model claude-sonnet-4-6 --dangerously-skip-permissions '{cmd}'"
high   = "claude --dangerously-skip-permissions '{cmd}'"

[cli]
tips = true

# [file_handlers]
# Choose which app opens each file type from the File Explorer (Enter / double-
# click). Keys are bare, lowercase extensions; values are `kind:value` specs:
#   "app:<id>"      open in a Plexi app by its registry id (e.g. text-editor)
#   "os"            hand to the OS default opener (open / xdg-open)
#   "cmd:<command>" reserved for a future release; falls back to "os" for now
# A handler here overrides an app's own file_types association. Unmapped types
# fall through to manifest associations, then builtin media players, then the OS.
# md   = "app:text-editor"
# txt  = "app:text-editor"
# json = "app:text-editor"
# py   = "os"

[marketplace]
# Hosted app catalog + CDN. Defaults point at the official plexiapp.com registry,
# so leave these unset to use it. Override only to point at a private registry.
# registry_url    = "https://plexiapp.com/registry/v1/index.json"
# cdn_url         = "https://plexiapp.com/registry/v1/packages"
# Publisher submission endpoint. Unset = `plexi app publish` prepares the package
# locally but does not upload it.
# submit_url      = "https://plexiapp.com/registry/v1/submit"
# Account/auth backend. Unset / "none" = login/signup fail closed. Accounts are
# only ever needed to publish or buy paid apps — free apps install without one.
# account_backend = "none"
# Accounts service host the device-flow login talks to. Defaults to plexiapp.com.
# account_url     = "https://plexiapp.com"
# Default email pre-filled by `plexi account login`.
# account_email   = "you@example.com"
```
