---
title: Configuration
description: Edit config.toml, choose a theme, wire AI models, and override shortcuts.
verified_version: "0.0.727"
order: 2
---

Plexi reads `config.toml` from the active channel profile. Alpha uses `~/.plexi-alpha/config.toml`, beta uses `~/.plexi-beta/config.toml`, main uses `~/.plexi/config.toml`, and PR builds use `~/.plexi-pr-<N>/config.toml`.

Changes take effect on the next launch unless a command says otherwise.

## Commands

```sh
plexi config edit
plexi config check
plexi config reset
```

`config edit` opens the active channel config. `config check` validates known keys and TOML syntax. `config reset` backs up the current file to `config.toml.bak`, then writes the built-in default template.

## Theme

Pick a preset at the top level:

```toml
theme_preset = "catppuccin-mocha"
```

Available presets: `catppuccin-mocha`, `catppuccin-latte`, `dracula`, `tokyo-night`, `gruvbox-dark`, `nord`, `solarized-dark`, `solarized-light`.

Override individual colors under `[theme]`:

```toml
[theme]
accent       = "#89b4fa"
terminal_bg  = "#292a44"
text_primary = "#cdd6f4"
```

Known color keys: `bg_darkest`, `bg_sidebar`, `bg_toolbar`, `terminal_bg`, `bg_hover`, `bg_sidebar_hover`, `bg_active`, `text_primary`, `text_dim`, `text_section`, `accent`, `border`, `foreground`, `background`, `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, `bright_black`, `bright_red`, `bright_green`, `bright_yellow`, `bright_blue`, `bright_magenta`, `bright_cyan`, `bright_white`, `bright_foreground`.

## AI

OpenRouter is the default cloud backend. Store the key in the global Plexi secret store:

```sh
plexi secret set openrouter-api-key --global
```

The broker also reads `OPENROUTER_API_KEY` from the host process environment:

```sh
export OPENROUTER_API_KEY=...
```

Do not paste API keys into `config.toml`.

```toml
[ai]
backend = "openrouter"

[ai.openrouter]
api_key_env  = "OPENROUTER_API_KEY"
model_low    = "qwen/qwen3.6-flash"
model_medium = "xiaomi/mimo-v2.5-pro"
model_high   = "anthropic/claude-fable-5"
```

Use Ollama for local models:

```toml
[ai]
backend = "ollama"

[ai.ollama]
host         = "http://localhost:11434"
model_low    = "llama3.2:3b"
model_medium = "llama3.3:70b"
model_high   = "qwq:32b"
```

Optional AI spend caps:

```toml
[ai]
per_app_daily_usd = 1.00
global_daily_usd  = 10.00
```

## Keybindings

Add only the shortcuts you want to override:

```toml
[keybindings]
toggle_command_palette = "cmd+p"
open_config            = "cmd+comma"
```

Modifiers: `cmd`, `shift`, `ctrl`, `alt`. Aliases: `command`, `control`, `opt`, `option`.

Keys: `a-z`, `0-9`, `enter`, `escape`, `tab`, `space`, `backspace`, `delete`, `up`, `down`, `left`, `right`, `open_bracket`, `close_bracket`, `backslash`, `slash`, `comma`, `period`, `equals`, `minus`.

Known actions: `quit`, `close_pane`, `toggle_command_palette`, `split_horizontal`, `split_vertical`, `split_right`, `split_down`, `swap_pane_left`, `swap_pane_down`, `swap_pane_up`, `swap_pane_right`, `send_pane_left`, `send_pane_down`, `send_pane_up`, `send_pane_right`, `navigate_left`, `navigate_down`, `navigate_up`, `navigate_right`, `new_tab`, `next_tab`, `prev_tab`, `first_tab`, `last_tab`, `nav_back`, `focus_history_forward`, `toggle_sidebar`, `toggle_zoom`, `toggle_shortcuts`, `rename_context`, `rename_pane`, `new_context`, `new_page_right`, `toggle_minimap`, `scroll_up`, `scroll_down`, `increase_font_size`, `decrease_font_size`, `open_file_browser`, `open_quick_note`, `open_config`, `reload_config`, `open_secrets_manager`, `force_reload_app`, `toggle_notification_modal`, `open_scratchpad`, `set_context_root_from_cwd`, `push_to_subcontext`, `new_child_context`, `hide_pane`, `park_context`, `open_notes_picker`.

Unknown keys or conflicting overrides log a warning at startup and keep the default binding.

## Notifications

```toml
[notifications]
enabled = true
focus_mode = false
interrupt_threshold = 100
```

`interrupt_threshold` controls which app notifications open the modal immediately. `100` means high and critical notifications interrupt; normal and low notifications queue silently.

## CLI

```toml
[cli]
tips = true
```

Set `tips = false` to hide contextual tips after CLI commands.

## Default config.toml

This is the built-in template Plexi writes when it creates or resets the active channel config.

```toml
# ╔══════════════════════════════════════════════════════════════╗
# ║  Plexi Configuration                                        ║
# ║  Changes take effect on next launch.                        ║
# ║  Full reference: https://plexiapp.com/docs/config           ║
# ╚══════════════════════════════════════════════════════════════╝
#
# Common commands:
#   plexi config edit   # open this file
#   plexi config check  # validate this file
#   plexi config reset  # back up and restore the default template

config_version = 1

font_size = 14.0

# Gap between panes in pixels. Default: 4. Range: 0-20.
# pane_gap = 4

# Font size for the pane name bar. Default: 11. Range: 6-32.
# pane_title_font_size = 11

# ── Theme ──────────────────────────────────────────────────────
# Presets: catppuccin-mocha, catppuccin-latte, dracula, tokyo-night, gruvbox-dark, nord, solarized-dark, solarized-light
theme_preset = "catppuccin-mocha"

# ── Confirmation Dialogs ───────────────────────────────────────
# confirm_quit requires triple Cmd+Q to exit (safer than a single press).
# confirm_close shows a dialog before Cmd+W closes a pane.
confirm_quit  = true
confirm_close = false

# ── Navigation ─────────────────────────────────────────────────
# Maximum depth of pane focus history (Cmd+[ / Cmd+]).
# focus_history_depth = 100

# ── OSC & Terminal Features ────────────────────────────────────────
# osc_pane_title applies OSC 0/1/2 title sequences from running
# processes as pane names.
osc_pane_title = true

# ── Notifications ──────────────────────────────────────────────
# The work-area modal is the one and only notification surface.
# Apps emit `ctx.notify(...)`, `ctx.notify_choice(...)`, or
# `ctx.notify_input(...)` and the modal renders each kind with
# keyboard-first navigation (Enter confirms, j/k or ↑↓ cycle
# options, 1-9 direct-select, Esc cancels when allowed).
[notifications]
# Master switch. If false, notifications are silently dropped at
# arrival — apps still send them, the modal never appears, and
# the queue stays empty.
# enabled = true

# Focus mode. When true, NO notification auto-surfaces regardless of
# priority. Everything queues silently; open Cmd+Shift+A to review.
# focus_mode = false

# Minimum priority that may auto-open the modal. Notifications below
# this value queue silently (badge ticks on the toolbar, Cmd+Shift+A
# reveals them). At or above it, arrival auto-opens the modal.
#
# Tiers (from plexi_sdk):
#   0   = PRIORITY_LOW       (background info)
#   50  = PRIORITY_NORMAL    (standard confirmations — "note saved")
#   100 = PRIORITY_HIGH      (needs attention soon)
#   200 = PRIORITY_CRITICAL  (interrupt-level)
#
# Default = 100: NORMAL and LOW queue silently, HIGH and CRITICAL
# interrupt. Set to 0 to auto-open everything. Set to 201 to match
# focus_mode = true (nothing auto-opens).
# interrupt_threshold = 100

# Esc vs Enter on the modal:
#   Enter (or option-select / input-submit) = acknowledge. Notification
#     is removed from the queue and the app receives NotifyAction.
#   Esc = defer. Modal closes but the notification stays in the queue —
#     open Cmd+Shift+A later to come back to it. No NotifyAction dispatched.
#   Required notifications (required = true) cannot be Esc'd.

[theme]
# Uncomment any line below to override the active preset.
# Full color reference: https://plexiapp.com/docs/config
#
# accent       = "#89b4fa"   # catppuccin-mocha defaults shown
# bg_darkest   = "#11111b"
# terminal_bg  = "#292a44"
# text_primary = "#cdd6f4"
# foreground   = "#e8e6ed"

# ── Plexi AI ───────────────────────────────────────────────────
# Apps with the `ai.query` capability route through this broker.
# Set the key with either:
#   plexi secret set openrouter-api-key --global
#   export OPENROUTER_API_KEY=...  # in your shell profile
# Do not paste API keys into this file.
[ai]
backend = "openrouter"   # "openrouter" (cloud) or "ollama" (local)

[ai.openrouter]
api_key_env  = "OPENROUTER_API_KEY"
model_low    = "qwen/qwen3.6-flash"
model_medium = "xiaomi/mimo-v2.5-pro"
model_high   = "anthropic/claude-fable-5"

# Local backend example:
# [ai.ollama]
# host         = "http://localhost:11434"
# model_low    = "llama3.2:3b"
# model_medium = "llama3.3:70b"
# model_high   = "qwq:32b"

# ── Logging ────────────────────────────────────────────────────
# [log]
# level = "info"          # error | warn | info | debug  (default: info)
# retention_days = 30     # days to keep dated log archives (default: 30)

# ── Keybindings ────────────────────────────────────────────────
# Override any default keybinding. Format: "modifier+key" (case-insensitive).
# Full action list: https://plexiapp.com/docs/config
#
# [keybindings]
# toggle_command_palette = "cmd+p"
# open_config            = "cmd+comma"

# ── Quick Note ─────────────────────────────────────────────────
# Cmd+0 opens a compose modal. Enter advances to the destination
# picker. Press a digit key to route instantly — no Enter needed.
# Submenu entries (options = [...]) expand on the parent keypress.
# Destination 0 (global backlog → ~/.plexi/backlog) is always
# available regardless of config.
#
# Tokens: {note} = your note text (shell-escaped)
#         {cwd}  = focused pane directory (shell-escaped)
#         {context_root} = active context's project root (shell-escaped, empty if unset)
# Security: only token values are shell-escaped. The command template itself is trusted
# (comes from config.toml). Do not substitute arbitrary user input outside of these tokens.

[[quick_note.destinations]]
key   = 1
label = "Backlog"
type  = "backlog"
path  = "~/.plexi/backlog"

[[quick_note.destinations]]
key     = 2
label   = "Ask Claude"
command = "plexi pane new 'claude -p {note}' --window"

[[quick_note.destinations]]
key   = 3
label = "GitHub issue"
# Submenu — each option runs a gh command in the background (hidden = true means no pane).
options = [
  { key = 1, label = "Bug",         command = "cd {cwd} && gh issue create --label bug --title {note} --body {note}",         hidden = true },
  { key = 2, label = "Enhancement", command = "cd {cwd} && gh issue create --label enhancement --title {note} --body {note}", hidden = true },
  { key = 3, label = "No label",    command = "cd {cwd} && gh issue create --title {note} --body {note}",                     hidden = true },
]

[[quick_note.destinations]]
key     = 4
label   = "Copy to clipboard"
command = "printf '%s' {note} | pbcopy"
hidden  = true

# Dynamic branch example — uncomment and customize:
# [[quick_note.destinations]]
# key          = 5
# label        = "Recent projects"
# children_cmd = "~/.plexi/scripts/top-projects.sh"
# # Script stdout format: one entry per line as  key|label|command
# # Example line: 1|PLEXI|cd ~/Documents/GitHub/PLEXI && plexi pane new 'claude -p {note}' --window

# ── Coding Agent Commands ──────────────────────────────────────
# Command templates used when Plexi skills dispatch coding agent panes.
# {cmd} is replaced with the prompt or slash command at dispatch time.
#
# Tiers:
#   low    → fast/cheap tasks (e.g. haiku)
#   medium → standard work   (e.g. sonnet)
#   high   → complex/autonomous tasks (default model, max permissions)
#
# Set these to your own aliases if you have them:
#   low    = "ch '{cmd}'"
#   medium = "c '{cmd}'"
#   high   = "ca '{cmd}'"
[agents]
low    = "claude --model claude-haiku-4-5 --dangerously-skip-permissions '{cmd}'"
medium = "claude --model claude-sonnet-4-6 --dangerously-skip-permissions '{cmd}'"
high   = "claude --dangerously-skip-permissions '{cmd}'"

# ── CLI Behavior ───────────────────────────────────────────────
[cli]
tips = true   # Set to false to suppress contextual tips after CLI commands.

# ══════════════════════════════════════════════════════════════
# Experimental / Beta
# ══════════════════════════════════════════════════════════════
# Flip any flag to true and restart to enable.
[beta]
# crt           = false    # Retro CRT scanlines + green phosphor tint
# ghost         = false    # Unfocused panes render at reduced opacity
# ghost_opacity = 0.9     # Opacity for unfocused panes (0.0–1.0). Setting this enables ghost automatically.
```
