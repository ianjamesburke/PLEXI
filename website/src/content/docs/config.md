---
title: Configuration
description: Edit config.toml, choose a theme, wire AI models, and override shortcuts.
order: 2
---

Plexi reads `config.toml` from the active channel profile. Alpha uses `~/.plexi-alpha/config.toml`, beta uses `~/.plexi-beta/config.toml`, main uses `~/.plexi/config.toml`, and PR builds use `~/.plexi-pr-<N>/config.toml`.

Changes take effect on the next launch unless a command says otherwise.

## Channel discipline

Existing configuration is preserved on every channel when Plexi is installed or upgraded. A fresh channel starts with the built-in template.

Beta is the staging channel for testing migrations and settings before promoting a release.

PR builds are isolated. A PR build reads `~/.plexi-pr-<N>/config.toml`; use the `plexi-pr-<N>` binary when checking a PR-specific config behavior.

## Commands

```sh
plexi config edit
plexi config check
plexi config reset
```

`config edit` opens the active channel config. `config check` validates known keys and TOML syntax. `config reset` backs up the current file to `config.toml.bak`, then writes the built-in default template.

A key with a wrong value type, a retired key, or an unknown key never discards the rest of the file: Plexi logs an error naming the file and key to `plexi.log`, skips that key, and loads every other setting. `config check`, and the Config Error notification shown when a saved config reloads, list the same keys. Only a TOML syntax error ignores the whole file.

## Reference

### Top-level (`config.toml`)

| Field | Type | Default | Description |
|---|---|---|---|
| `core_only` | bool | — | Disable all plugins for this host session. Profile-only; restart required. |
| `config_version` | integer | — | — |
| `font_size` | float | — | — |
| `ui_scale` | float | 1.0 | App-wide UI scale (browser-style zoom). Default 1.0; clamped on use. |
| `pane_gap` | float | 4.0 | Inter-pane gap width in pixels. Default: 4.0. Clamped to [0.0, 20.0]. |
| `pane_corner_radius` | u8 | 6 | Pane corner radius in points. Default: 6 (the shared pane style token). |
| `pane_title_font_size` | float | 11.0 | Pane title bar font size. Default: 11.0. Clamped to [6.0, 32.0]. |
| `osc_pane_title` | bool | true | Apply OSC 0/1/2 title sequences from terminal processes as pane names. Defaults to true; set false to keep terminal pane names manual-only. |
| `theme_preset` | string | — | — |
| `attention` | AttentionConfig | — | — |
| `confirm_quit` | bool | true | Set to false to quit immediately on Cmd+Q without triple-press confirmation (default: true). |
| `confirm_close` | bool | true | Set to false to close panes immediately on Cmd+W without a confirmation dialog (default: true). |
| `confirm_context_close` | bool | true | Set to false to close contexts immediately on Cmd+Shift+W without a confirmation dialog (default: true). |
| `focus_history_depth` | integer | — | — |
| `cloud` | CloudConfig | — | Optional cloud link (`[cloud]`). Local use never reads this. See [`CloudConfig`]. |

### Theme (`[theme]`)

Pick a preset and optionally override individual colors. Individual color fields override preset values when both are set.

Available presets: `catppuccin-mocha`, `catppuccin-latte`, `dracula`, `tokyo-night`, `tokyo-day`, `gruvbox-dark`, `gruvbox-light`, `nord`, `solarized-dark`, `solarized-light`, `matrix`, `bios`, `plexi-night`, `plexi-dusk`, `plexi-dawn`, `plexi-day`.

| Field | Type | Default | Description |
|---|---|---|---|
| `preset` | string | — | Theme preset name. Applied first; individual color fields override the preset. |
| `state_preset` | string | okabe-ito | Agent states: default, okabe-ito, high-contrast, mono. Default: okabe-ito. |
| `state_working` | string | — | — |
| `state_waiting` | string | — | — |
| `state_blocked` | string | — | — |
| `state_needs_you` | string | — | — |
| `state_interrupted` | string | — | — |
| `state_error` | string | — | — |
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

### Terminal (`[terminal]`)

`minimum_contrast` keeps text readable on every theme. Each text cell's foreground is checked against its background and, when the WCAG contrast ratio is below the minimum, moved toward black or white by the smallest amount that reaches it. Hidden text stays hidden, and emoji and images are untouched. Applies live on config reload.

| Field | Type | Default | Description |
|---|---|---|---|
| `minimum_contrast` | float | 3.0 | Minimum WCAG contrast ratio between each text cell's foreground and its background, 1.0 to 21.0. Colors programs choose for themselves are moved toward black or white until they reach it. 1.0 turns the adjustment off. Default 3.0. Applies live on config reload. |

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
| `enabled` | bool | true | Master switch. If false, incoming notifications are silently dropped — plugins still send them, but the modal never appears and the queue stays empty. Defaults to true. |
| `focus_mode` | bool | false | Focus mode. When true, no notification auto-surfaces. Everything queues silently; the user reviews via Cmd+Shift+A. Defaults to false. |
| `sound` | string | — | Path to an audio file (WAV, MP3, FLAC, OGG) played once when a notification arrives, so an agent waiting on input can pull a human back while Plexi is in the background. Unset (the default) means no sound; an explicitly empty string also means no sound, so a workspace overlay can set `sound = ""` to silence a globally configured cue. The cue only fires for a notification that may interrupt: `enabled = false` drops it before the cue, `focus_mode = true` suppresses it, and an invisible notification arrives silently. |

### AI (`[ai]`)

API keys are NOT stored in `config.toml`. Export `OPENROUTER_API_KEY` in your shell profile (`~/.zshrc`, `~/.zprofile`, etc.), or store it in the Plexi secret store:

```sh
plexi secret set openrouter-api-key --global
```

| Field | Type | Default | Description |
|---|---|---|---|
| `per_plugin_daily_usd` | float | $1.00 | Per-plugin daily spend cap in USD. Default $1.00. |
| `global_daily_usd` | float | $10.00 | Global daily spend cap across all plugins in USD. Default $10.00. |
| `client` | string | — | Default ledger client tag for Assistant and agent runs that do not set one. Free-form (`narrative`, `du`, `personal`). A blank or missing value leaves the row's client null. |

### Model modes (`[models.fast]`, `[models.slow]`)

pi-plexi and the host AI broker read resolved targets with `plexi config get --json models`. Each field is optional and falls back to the built-in default independently. OpenRouter keys use the existing `OPENROUTER_API_KEY` secret lookup.

| Field | Type | Default | Description |
|---|---|---|---|
| `provider` | string | — | Model registry provider. Must be non-empty and contain no whitespace. |
| `model` | string | — | Provider model identifier. Must be non-empty and contain no whitespace. |
| `thinking` | string | — | Pi thinking level: `off`, `minimal`, `low`, `medium`, `high`, or `xhigh`. |

### Agents (`[agents]`)

Command templates for dispatching coding agents. `{cmd}` is replaced with the prompt or slash command at dispatch time.

| Field | Type | Default | Description |
|---|---|---|---|
| `os_notifications` | bool | — | Raise silent macOS attention notifications while Plexi is unfocused (default true). Uses provisional authorization without a permission dialog; enable banners in macOS notification settings if desired. Disabled in tests; other platforms log only. |
| `stalled_after_secs` | integer | — | Idle ephemeral worker stall threshold in seconds (default 120). |
| `stale_failure_hours` | integer | — | Age threshold in hours after which failed runs collapse into the older fleet group (default 12). |
| `low` | string | claude haiku with --dangerously-skip-permissions | Fast/cheap tasks. Default: claude haiku with --dangerously-skip-permissions. |
| `medium` | string | claude sonnet with --dangerously-skip-permissions | Standard work. Default: claude sonnet with --dangerously-skip-permissions. |
| `high` | string | claude with --dangerously-skip-permissions | Complex/autonomous tasks. Default: claude with --dangerously-skip-permissions. |
| `restore_mode` | AgentRestoreMode | — | Startup restoration mode for saved agent sessions: `"lazy"` (default), `"eager"`, or `"off"`. |
| `standby_scrollback_lines` | integer | — | Number of scrollback lines saved for a lazy-restored Standby pane (default 20). |

### Keybindings (`[keybindings]`)

Omitted keys keep their defaults. `plexi config check` reports chord conflicts. The legacy name `force_reload_plugin` still maps to `force_reload_view`.

In the command palette, open a result's Actions menu and choose **Assign shortcut in config**. The editor shows the command ID to set under `[keybindings]`. The same IDs work in workspace config.

Modifier keys: `cmd` (alias: `command`), `shift`, `ctrl` (alias: `control`), `alt` (aliases: `opt`, `option`).

Key names: `a`–`z`, `0`–`9`, `enter`, `escape`, `tab`, `space`, `backspace`, `delete`, `up`, `down`, `left`, `right`, `open_bracket`, `close_bracket`, `backslash`, `slash`, `comma`, `period`, `equals`, `minus`.

Each value is a string like `"cmd+p"` or `"cmd+shift+w"`.

| Command ID | Description |
|---|---|
| `quit` | Quit the application using the configured confirmation |
| `close_pane` | Close the focused pane (dismisses overlays and view back-stacks first) |
| `toggle_command_palette` | Command palette |
| `open_quick_note` | Capture a note in the current context |
| `open_agents` | Review tasks, workers and blocking reasons |
| `next_blocked_agent` | Focus the next agent that needs attention |
| `focus_approval` | Focus the permission banner for a keyboard decision |
| `toggle_attention_queue` | Open or close the attention queue list |
| `zoom_in` | Wrap selected panes, enter a portal, or toggle pane zoom |
| `context_zoom_out` | Close the open overlay, else return to the parent context |
| `close_context` | Close the selected context (confirmation required) |
| `split_vertical` | Open a new terminal below |
| `split_horizontal` | Open a new terminal to the right |
| `swap_pane_left` | Swap the focused pane with its neighbor left |
| `swap_pane_down` | Swap the focused pane with its neighbor down |
| `swap_pane_up` | Swap the focused pane with its neighbor up |
| `swap_pane_right` | Swap the focused pane with its neighbor right |
| `send_pane_left` | Move the focused pane without following it left |
| `send_pane_down` | Move the focused pane without following it down |
| `send_pane_up` | Move the focused pane without following it up |
| `send_pane_right` | Move the focused pane without following it right |
| `new_tab` | Create a new tab in the selected context |
| `next_tab` | Focus the next tab |
| `prev_tab` | Focus the previous tab |
| `move_context_up` | Move the active context up in the sidebar |
| `move_context_down` | Move the active context down in the sidebar |
| `navigate_left` | Move focus left |
| `navigate_down` | Move focus down |
| `navigate_up` | Move focus up |
| `navigate_right` | Move focus right |
| `toggle_sidebar` | Show or hide the context sidebar |
| `toggle_zoom` | Zoom or restore the selected pane (dives into an agent's children first) |
| `toggle_shortcuts` | Show keyboard shortcuts reference |
| `rename_context` | Rename the selected context |
| `rename_pane` | Rename the selected pane |
| `split_down` | Split down: a new terminal, or a new copy of the focused view |
| `split_right` | Split right: a new terminal, or a new copy of the focused view |
| `push_to_subcontext` | Move the focused pane into a new child context |
| `new_child_context` | Create and enter an empty child context |
| `new_context` | Create a new context |
| `new_page_right` | Create a new window in the current context |
| `toggle_minimap` | Show or hide the pane minimap |
| `scroll_up` | Scroll the focused pane up |
| `scroll_down` | Scroll the focused pane down |
| `increase_font_size` | Make the focused pane's text larger |
| `decrease_font_size` | Make the focused pane's text smaller |
| `reset_font_size` | Restore the focused pane's text size to default |
| `ui_scale_up` | Make the whole interface larger |
| `ui_scale_down` | Make the whole interface smaller |
| `ui_scale_reset` | Restore the interface to 100% |
| `open_file_browser` | Browse files and directories in the focused context |
| `open_config` | Choose a global or workspace config to edit |
| `reload_config` | Reload global and active-workspace configuration |
| `open_secrets_manager` | Manage scoped credentials, usage and grants |
| `open_assistant` | Open or focus the Plexi Assistant |
| `force_reload_view` | Restart the focused process-backed view |
| `set_context_root_from_cwd` | Use the focused pane’s current directory as the context root |
| `park_context` | Park or restore the selected context |
| `open_scratchpad` | Open a fresh scratch note editor |
| `open_notes_picker` | Open the notes picker for global and context notes |
| `next_context` | Focus the next context |
| `prev_context` | Focus the previous context |
| `nav_back` | Go back in pane navigation or focus history |
| `focus_history_forward` | Return to the next pane in focus history |
| `select_pane_left` | Extend the selection left |
| `select_pane_down` | Extend the selection down |
| `select_pane_up` | Extend the selection up |
| `select_pane_right` | Extend the selection right |
| `open_changelog` | Show what changed in this version |
| `open_global_config` | Edit the channel-wide Plexi config file |
| `open_workspace_config` | Edit config for the active workspace scope |
| `state_preset_default` | Apply agent state colors; shapes always identify the state |
| `state_preset_okabe_ito` | Apply agent state colors; shapes always identify the state |
| `state_preset_high_contrast` | Apply agent state colors; shapes always identify the state |
| `state_preset_mono` | Apply agent state colors; shapes always identify the state |
| `edit_commands` | Edit commands.toml for the active workspace or global profile |
| `switch_commands_scope` | Switch commands editor between workspace and global scope |
| `open_attention` | Open the attention queue for approvals, questions and host notices |
| `setup_cli` | Install the ~/.local/bin launcher, completions, and PATH registration |
| `ui_upcycle_git` | Open git's curated command list and forms |
| `ui_upcycle_cargo` | Open cargo's curated command list and forms |
| `ui_upcycle_gh` | Open gh's curated command list and forms |
| `check_for_updates` | Look for a newer Plexi build on this channel now |
| `restart_to_update` | Restart into the downloaded build after confirmation |
| `close_selected_panes` | Close every selected pane, asking about unsaved work one pane at a time |
| `cancel_pane_selection` | Clear the active window’s pane selection |
| `switch_context_1` | Focus a sidebar context by its position |
| `switch_context_2` | Focus a sidebar context by its position |
| `switch_context_3` | Focus a sidebar context by its position |
| `switch_context_4` | Focus a sidebar context by its position |
| `switch_context_5` | Focus a sidebar context by its position |
| `switch_context_6` | Focus a sidebar context by its position |
| `switch_context_7` | Focus a sidebar context by its position |
| `switch_context_8` | Focus a sidebar context by its position |
| `switch_context_9` | Focus a sidebar context by its position |

### User command shortcuts (`[keys]`)

Bind chords to user commands defined in `~/.plexi-<channel>/commands.toml` or `<repo>/.plexi-<channel>/commands.toml` (`~/.plexi/commands.toml` and `<repo>/.plexi/commands.toml` on stable) using `"<chord>" = "run:<name>"` (for example `"cmd+shift+p" = "run:play"`). Keybindings resolve the workspace from the adopted/host workspace root, falling back to the active context root (`run_user_command_by_name`), with precedence workspace `commands.toml` > global `commands.toml` > global `scripts/`, and re-read `commands.toml` on every invocation without a restart. Reserved Plexi shortcuts are rejected by `plexi config check`.

### CLI (`[cli]`)

| Field | Type | Default | Description |
|---|---|---|---|
| `tips` | bool | `true` | Print contextual tips after CLI commands. Default: `true`. Set to `false` to suppress. |

### Permissions (`[permissions]`)

| Field | Type | Default | Description |
|---|---|---|---|
| `soft_grant_days` | integer | 30 | Expiry window in days for soft grants (`--grant tool=allow:soft`). After this window, a soft grant reverts to `ask`. Default: 30. |

### File handlers (`[file_handlers]`)

Choose which view opens each file type from the File Explorer. Keys are bare lowercase extensions (e.g. `md`, not `.md`). Values are handler specs:

| Value | Behavior |
|---|---|
| `app:<id>` | Stored config spelling: open a plugin default view by its registry id (e.g. `app:text-editor`) |
| `os` | Hand to the OS default opener (`open` / `xdg-open`) |

Any other value, including `cmd:<command>`, is reported as an invalid setting and ignored. A handler here overrides a plugin's own `file_types` association. Unmapped extensions fall through to manifest associations, then builtin media players, then the OS default.

### Marketplace (`[marketplace]`)

Marketplace configuration is beta-gated and is not part of stable v1. Use it only in a beta or explicit worktree channel while testing publishing flows. All fields are optional.

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

# Start with no plugins. Profile-only; restart the host after changing.
core_only = false

font_size = 14.0
# ui_scale = 1.0               # app-wide UI scale (0.6-2.0); Cmd+= / Cmd+- / Cmd+Shift+0
# pane_gap = 4                 # inter-pane gap in pixels (0-20)
# pane_corner_radius = 6        # shared pane corner radius in points (0-255)
# pane_title_font_size = 12    # pane title bar font size (6-32)

confirm_quit  = true
confirm_close = false
confirm_context_close = true

# focus_history_depth = 100
osc_pane_title = true

[theme]
# Agent state colors retain distinct shapes in every preset.
state_preset = "okabe-ito" # default | okabe-ito | high-contrast | mono
# state_working = "#0072b2"
# state_waiting = "#e69f00"
# state_blocked = "#cc79a7"
# state_needs_you = "#d55e00"
# state_interrupted = "#009e73"
# state_error = "#f0e442"
preset = "plexi-dusk"
# Presets: catppuccin-mocha, catppuccin-latte, dracula, tokyo-night, tokyo-day, gruvbox-dark, gruvbox-light, nord, solarized-dark, solarized-light, matrix, bios, plexi-night, plexi-dusk, plexi-dawn, plexi-day
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

[terminal]
# Lowest allowed contrast ratio (1.0-21.0) between text and its cell background.
# Colors a program picks for itself are moved toward black or white until they
# reach it. 1.0 turns it off. 3.0 is the WCAG floor for readable text.
minimum_contrast = 3.0

[notifications]
# enabled = true
# focus_mode = false
# sound = "/System/Library/Sounds/Ping.aiff"   # cue played on arrival; unset = silent

[ai]
# client = "personal"          # default AI ledger tag; a run can override it

# FAST/SLOW targets for pi-plexi and the host AI broker. Each field may be
# overridden independently; omitted fields use these defaults. OpenRouter API
# keys are resolved as OPENROUTER_API_KEY through Plexi secrets or the process.
# [models.fast]
# provider = "openrouter"
# model = "deepseek/deepseek-v4.1-flash"
# thinking = "low"             # off | minimal | low | medium | high | xhigh
# [models.slow]
# provider = "openrouter"
# model = "openai/gpt-6-astra"
# thinking = "high"

# [log]
# level = "info"               # error | warn | info | debug — applies live on save, no restart
# retention_days = 30          # dated plexi-YYYY-MM-DD.log archives. The live plexi.log stays.

# [cloud]
# Local use never needs an account. Signing in only links this desktop to
# cloud features. Unset, local ledger rows and assistant
# conversations are kept.
# retain_local_history = false

# [experiments]
# Features outside the stable v1 surface. All off by default on every channel.
# Manage with `plexi experiments list|enable|disable`. Global config only.
# assistant = true
# tree_atlas = true # Context tree overview from the root with Cmd+Escape

# Pane selection: Cmd+Shift+H/J/K/L; wrap selection: Cmd+Enter.
# Next/previous tab: Cmd+] / Cmd+[. Context/history navigation is unbound.
# [keybindings]
# open_changelog = "cmd+alt+c" # No default. Find other IDs in the palette Actions menu.
# select_pane_left = "cmd+shift+h"
# select_pane_down = "cmd+shift+j"
# select_pane_up = "cmd+shift+k"
# select_pane_right = "cmd+shift+l"
# open_agents = "cmd+alt+a"
# next_blocked_agent = "cmd+alt+b"
# focus_approval = "cmd+alt+p"
# toggle_command_palette = "cmd+p"
# open_config            = "cmd+comma"
# close_context          = "cmd+shift+w"

# [keys]
# Bind a chord to a user command from ~/.plexi-<channel>/commands.toml or
# <repo>/.plexi-<channel>/commands.toml (stable: ~/.plexi/commands.toml,
# <repo>/.plexi/commands.toml) using "<chord>" = "run:<name>":
# "cmd+shift+p" = "run:play"

[agents]
# Silent desktop attention alerts while Plexi is not focused.
os_notifications = true
stalled_after_secs = 120
stale_failure_hours = 12
low    = "claude --model claude-haiku-4-5 --dangerously-skip-permissions '{cmd}'"
medium = "claude --model claude-sonnet-4-6 --dangerously-skip-permissions '{cmd}'"
high   = "claude --dangerously-skip-permissions '{cmd}'"
# restore_mode = "lazy"        # "lazy" (active context resumes, background enters Standby), "eager", or "off"
# standby_scrollback_lines = 20 # scrollback rows saved and shown in a Standby pane

[cli]
tips = true

# [permissions]
# soft_grant_days = 30         # expiry window in days for `--grant tool=allow:soft` grants; after expiry they revert to `ask`

# [file_handlers]
# Choose which plugin opens each file type from the File Explorer (Enter / double-
# click). Keys are bare, lowercase extensions; values are `kind:value` specs:
#   "app:<id>"      stored config spelling: open a plugin default view by its registry id (e.g. text-editor)
#   "os"            hand to the OS default opener (open / xdg-open)
# A handler here overrides a plugin's own file_types association. Unmapped types
# fall through to manifest associations, then builtin media players, then the OS.
# md   = "app:text-editor"
# txt  = "app:text-editor"
# json = "app:text-editor"
# py   = "os"

[marketplace]
# Hosted plugin catalog + CDN. Defaults point at the official plexiapp.com registry,
# so leave these unset to use it. Override only to point at a private registry.
# registry_url    = "https://plexiapp.com/registry/v1/index.json"
# cdn_url         = "https://plexiapp.com/registry/v1/packages"
# Publisher submission endpoint. Unset = `plexi plugin publish` prepares the package
# locally but does not upload it.
# submit_url      = "https://plexiapp.com/registry/v1/submit"
# Account/auth backend. Unset / "none" = login/signup fail closed. An account is
# optional: local use never needs one. A session only links this desktop to
# cloud features.
# account_backend = "none"
# Accounts service host the device-flow login talks to. Defaults to plexiapp.com.
# account_url     = "https://plexiapp.com"
# Default email pre-filled by `plexi account login`.
# account_email   = "you@example.com"

[attention]
style = "normal" # off | subtle | normal | loud
intensity = 1.0 # 0.0..1.0, scales halo and border opacity
dock = "badge" # none | badge | bounce-once | bounce-until-focused
```
