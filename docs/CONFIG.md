Plexi reads config.toml from the active channel profile. Alpha uses ~/.plexi-alpha/config.toml, beta uses ~/.plexi-beta/config.toml, main uses ~/.plexi/config.toml, and PR builds use ~/.plexi-pr-<N>/config.toml.

Changes take effect on the next launch unless a command says otherwise.

## Channel discipline

Alpha config stays default. `just install` refreshes `~/.plexi-alpha/config.toml` from the built-in template, and PR builds seed from alpha. Do not use alpha for personal overrides you expect to keep.

Beta config is the staging ground. `~/.plexi-beta/config.toml` is not reset by alpha installs, so it is the right place to test migrations, advanced settings, and personal config before promoting a release.

PR builds are isolated. A PR build reads `~/.plexi-pr-<N>/config.toml`; use the `plexi-pr-<N>` binary when checking a PR-specific config behavior.

## Commands

```
plexi config list                  Print all known keys with type, value, and description.
plexi config list --json           Same, as a JSON array.
plexi config get KEY               Print the resolved effective value of a single key.
plexi config set KEY=VALUE ...     Write one or more keys in-place (e.g. theme.preset=dracula).
plexi config edit                  Open config.toml in $EDITOR.
plexi config check                 Validate known keys and TOML syntax.
plexi config reset                 Back up config.toml → config.toml.bak and write the default template.
```

`config list` is the canonical way to discover valid keys before writing. `config set` resolves scope the same way as the rest of config: workspace when inside a workspace, global otherwise. Override with `-g`/`--global` or `-w`/`--workspace`.

## Theme

Pick a preset and optionally override individual colors under `[theme]`:

```toml
[theme]
preset       = "catppuccin-mocha"
accent       = "#89b4fa"
terminal_bg  = "#292a44"
text_primary = "#cdd6f4"
```

Available presets: catppuccin-mocha, catppuccin-latte, dracula, tokyo-night, tokyo-day, gruvbox-dark, gruvbox-light, nord, solarized-dark, solarized-light.

Known color keys: bg_darkest, bg_sidebar, bg_toolbar, terminal_bg, bg_hover, bg_sidebar_hover, bg_active, text_primary, text_dim, text_section, accent, border, foreground, background, black, red, green, yellow, blue, magenta, cyan, white, bright_black, bright_red, bright_green, bright_yellow, bright_blue, bright_magenta, bright_cyan, bright_white, bright_foreground.

## AI

OpenRouter is the default cloud backend. Store the key in the global Plexi secret store:

```
plexi secret set openrouter-api-key --global
```

The broker also reads `OPENROUTER_API_KEY` from the host process environment:

```
export OPENROUTER_API_KEY=...
```

Do not paste API keys into config.toml.

```toml
[ai]
backend = "openrouter"

[ai.openrouter]
api_key_env  = "OPENROUTER_API_KEY"
model_low    = "qwen/qwen3.6-flash"
model_medium = "xiaomi/mimo-v2.5"
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

Use `local` for any other OpenAI-compatible chat-completions server (e.g. a Meridian proxy). `base_url` is required. `api_key_env` is optional: when set, the named environment variable must hold the key; when unset, no auth header is sent.

```toml
[ai]
backend = "local"

[ai.local]
base_url     = "http://127.0.0.1:3456"
model_low    = "claude-haiku-4-5"
model_medium = "claude-opus-5"
model_high   = "claude-fable-5"
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

Modifiers: cmd, shift, ctrl, alt. Aliases: command, control, opt, option.

Keys: a-z, 0-9, enter, escape, tab, space, backspace, delete, up, down, left, right, open_bracket, close_bracket, backslash, slash, comma, period, equals, minus.

Known actions: quit, close_pane, toggle_command_palette, split_horizontal, split_vertical, split_right, split_down, swap_pane_left, swap_pane_down, swap_pane_up, swap_pane_right, send_pane_left, send_pane_down, send_pane_up, send_pane_right, navigate_left, navigate_down, navigate_up, navigate_right, new_tab, next_tab, prev_tab, next_context, prev_context, move_context_up, move_context_down, nav_back, focus_history_forward, toggle_sidebar, toggle_zoom, toggle_shortcuts, rename_context, rename_pane, new_context, new_page_right, toggle_minimap, scroll_up, scroll_down, increase_font_size, decrease_font_size, open_file_browser, open_quick_note, open_config, reload_config, open_secrets_manager, open_assistant, force_reload_app, toggle_notification_modal, open_scratchpad, push_to_subcontext, new_child_context, set_context_root_from_cwd, hide_pane, park_context, open_notes_picker, close_context.

Unknown keys or conflicting overrides log a warning at startup and keep the default binding.

## Notifications

```toml
[notifications]
enabled = true
focus_mode = false
interrupt_threshold = 100
```

`interrupt_threshold` controls which app notifications open the modal immediately. 100 means high and critical notifications interrupt; normal and low notifications queue silently.

## CLI

```toml
[cli]
tips = true
```

Set `tips = false` to hide contextual tips after CLI commands.

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
# Presets: catppuccin-mocha, catppuccin-latte, dracula, tokyo-night, tokyo-day, gruvbox-dark, gruvbox-light, nord, solarized-dark, solarized-light
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
# interrupt_threshold = 100    # 0=LOW 50=NORMAL 100=HIGH 200=CRITICAL

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

[marketplace]
# Hosted app catalog + CDN. Defaults point at the official plexiapp.com registry,
# so leave these unset to use it. Override only to point at a private registry.
# registry_url    = "https://plexiapp.com/registry/v1/index.json"
# cdn_url         = "https://plexiapp.com/registry/v1/packages"
# Publisher submission endpoint. Unset = `plexi app publish` prepares the package
# locally but does not upload it.
# submit_url      = "https://plexiapp.com/registry/v1/submit"
# Account/auth backend. "plexi" enables plexiapp.com accounts; unset / "none" =
# login fails closed. Accounts are only ever needed to publish or buy paid apps —
# free apps install without one.
# account_backend = "plexi"
# Accounts service base URL. Unset = the official plexiapp.com service. Override
# only to point `plexi account login` at a private deployment.
# account_url     = "https://plexiapp.com"
# Default email pre-filled by `plexi account login`.
# account_email   = "you@example.com"
```
