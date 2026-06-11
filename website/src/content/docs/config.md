---
title: Configuration
description: Edit config.toml, choose a theme, wire AI models, and override shortcuts.
verified_version: "0.0.717"
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
