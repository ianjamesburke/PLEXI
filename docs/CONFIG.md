Plexi reads config.toml from the active channel profile. Alpha uses ~/.plexi-alpha/config.toml, beta uses ~/.plexi-beta/config.toml, main uses ~/.plexi/config.toml, and PR builds use ~/.plexi-pr-<N>/config.toml.

Changes take effect on the next launch unless a command says otherwise.

## Channel discipline

Alpha config stays default. `just install` refreshes `~/.plexi-alpha/config.toml` from the built-in template, and PR builds seed from alpha. Do not use alpha for personal overrides you expect to keep.

Beta config is the staging ground. `~/.plexi-beta/config.toml` is not reset by alpha installs, so it is the right place to test migrations, advanced settings, and personal config before promoting a release.

PR builds are isolated. A PR build reads `~/.plexi-pr-<N>/config.toml`; use the `plexi-pr-<N>` binary when checking a PR-specific config behavior.

## Experiments

Features outside the stable v1 surface are experiments. Every experiment is off by default on every channel, including alpha. Channels decide update cadence and profile directory, never features.

```toml
[experiments]
assistant = true
```

Manage the table with `plexi experiments list`, `plexi experiments enable <name>`, and `plexi experiments disable <name>`. Those edit config.toml in place and keep your comments and other keys. When the host reads an experiment at runtime, enabling it prints `restart required: run `plexi host restart``. An unknown name is a config error that lists the valid names; `plexi experiments`, `plexi config`, and `--help` still run so you can fix it.

Experiments are read from the global config only. A workspace `.plexi` config that sets `[experiments]` has no effect, so a cloned repository can never switch on the relay, cloud, or agent surfaces for you.

`plexi skill install` and `plexi completions` snapshot the enabled set when they run; rerun them after changing an experiment.

## Commands

```
plexi config list                  Print all known keys with type, value, and description.
plexi config list --json           Same, as a JSON array.
plexi config get KEY               Print a value from the selected config file.
plexi config get --json models     Print resolved pi-plexi FAST/SLOW targets as JSON.
plexi config set KEY=VALUE ...     Write one or more keys in-place (e.g. theme.preset=dracula).
plexi config edit                  Open config.toml in $EDITOR.
plexi config check                 Validate known keys and TOML syntax.
plexi config reset                 Back up config.toml → config.toml.bak and write the default template.
```

`config list` is the canonical way to discover valid keys before writing. Every `config` subcommand defaults to the workspace config when inside a workspace and the channel-global config otherwise. Override with `-g`/`--global` or `-w`/`--workspace`.

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

Agent state colors are independent of the overall theme and default to the `okabe-ito` preset. Search **Settings: agent states** in the command palette to select a preset, or set `theme.state_preset`. State glyphs remain distinct even in the mono preset. Individual `theme.state_*` overrides take precedence; legacy `pip_working`, `pip_idle`, and `pip_blocked` overrides remain supported.

The `[attention]` table controls NeedsYou emphasis and Dock delivery. Changes apply through the config watcher. Reduce Motion keeps the attention ring static; Working alone does not animate. For accepted values and defaults, see the [generated configuration reference](../website/src/content/docs/config.md).

## AI

## Pi FAST/SLOW model modes

`pi-plexi` reads the resolved targets with `plexi config get --json models`.
Configure either mode globally or in workspace config; omitted fields inherit
the built-in value. See the generated configuration reference for field details
and allowed thinking levels.

```toml
[models.fast]
model = "deepseek/deepseek-v4.1-flash"
thinking = "low"

[models.slow]
model = "openai/gpt-6-astra"
thinking = "high"
```

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
[models.fast]
provider = "openrouter"
model = "deepseek/deepseek-v4.1-flash"
thinking = "low"

[models.slow]
provider = "openrouter"
model = "openai/gpt-6-astra"
thinking = "high"
```

`[models.fast]` and `[models.slow]` replace the retired `ai.backend` and every `ai.openrouter`, `ai.ollama`, and `ai.local` model key. OpenRouter uses the existing `OPENROUTER_API_KEY` secret lookup; no API-key config knob is supported.

Optional AI spend caps, and the default ledger client tag. A run can override `client`; when both are unset the ledger row stores null. `kind` is not a config key — it defaults to `output` per run.

```toml
[ai]
per_app_daily_usd = 1.00
global_daily_usd  = 10.00
# client = "personal"
```

## Keybindings

Add only the shortcuts you want to override:

```toml
[keybindings]
toggle_command_palette = "cmd+p"
open_config            = "cmd+comma"
```

`ui_scale_up` / `ui_scale_down` (Cmd+= / Cmd+-) step one global UI scale that
scales everything together: terminal text, UI text, spacing, cards, overlays and
modals. `ui_scale_reset` (Cmd+Shift+0) returns to 100%. The scale persists as
top-level `ui_scale` (0.6-2.0, default 1.0; out-of-range values are clamped).
`increase_font_size` (Cmd+=) / `decrease_font_size` (Cmd+-) adjust the focused
pane's text size independently of global interface scale.

`focus_approval` defaults to Cmd+Alt+P and focuses the oldest pending permission
banner. After its dimmed hint arms, `a` allows once, `d` denies, and Escape leaves
the request pending and returns keyboard focus. Typing in a pane never decides a
request. Shift+A opens confirmation for the banner's **Always deny** choice;
Enter confirms after a fresh arm delay, and Escape leaves it pending. Session
approval has no default decision key. See `ApprovalKeyboard` in
`src/app/approval_keyboard.rs` for the delay and focus contract.

Modifiers: cmd, shift, ctrl, alt. Aliases: command, control, opt, option.

Keys: a-z, 0-9, enter, escape, tab, space, backspace, delete, up, down, left, right, open_bracket, close_bracket, backslash, slash, comma, period, equals, minus.

Known actions: focus_approval, quit, close_pane, toggle_command_palette, split_horizontal, split_vertical, split_right, split_down, swap_pane_left, swap_pane_down, swap_pane_up, swap_pane_right, send_pane_left, send_pane_down, send_pane_up, send_pane_right, navigate_left, navigate_down, navigate_up, navigate_right, new_tab, next_tab, prev_tab, next_context, prev_context, move_context_up, move_context_down, nav_back, focus_history_forward, toggle_sidebar, toggle_zoom, toggle_shortcuts, rename_context, rename_pane, new_context, new_page_right, toggle_minimap, scroll_up, scroll_down, increase_font_size, decrease_font_size, reset_font_size, ui_scale_up, ui_scale_down, ui_scale_reset, open_file_browser, open_quick_note, open_config, reload_config, open_secrets_manager, open_assistant, force_reload_app, toggle_notification_modal, open_scratchpad, push_to_subcontext, new_child_context, set_context_root_from_cwd, hide_pane, park_context, open_notes_picker, close_context.

Unknown keys or conflicting overrides log a warning at startup and keep the default binding.

## Notifications

```toml
[notifications]
enabled = true
focus_mode = false
# sound = "/System/Library/Sounds/Ping.aiff"
```

`sound` is an optional path to an audio file (WAV, MP3, FLAC, OGG) played once when a notification arrives, so an agent waiting on input can pull you back while Plexi is in the background. Unset means no sound; `sound = ""` also means no sound, so a workspace overlay can silence a globally configured cue. The cue only fires for a notification that would also be allowed to open the modal: `enabled = false` drops the notification before the cue ever runs, `focus_mode = true` suppresses it, and a notification that is not visible in the active context arrives silently — the cue and the modal share one interruption decision, so nothing ever beeps with nothing on screen.

### Notification scope

Scope decides which contexts a notification is visible in: `window` (only the originating window), `context` (only the context that produced it), or `global` (everywhere).

**A notification that does not request a scope is `context`.** It belongs to the context that produced it; following the user into unrelated contexts is the deliberate, opt-in case. Precedence, highest first:

1. An explicitly requested scope — `plexi notify --scope global|context|window`.
2. For app panes, the app's own `manifest.toml` `[launch] notification_scope`, which is a per-app policy declared by the app author and defaults to `window`.
3. Otherwise the shared default, `context`.

A `context`- or `window`-scoped notification needs a sender to attach to. `plexi notify` stamps the caller's own identity (`PLEXI_CONTEXT_ID` / `PLEXI_PANE_ID`) onto the request; the host attaches the notification to that context — never to whichever context happens to be active when the request is handled. When the sender cannot support the requested scope, the host resolves to the nearest wider scope it can honor (logged): `window` with a resolvable context but no live window narrows to `context`; a caller with no resolvable identity at all (a script running outside any pane with `PLEXI_SOCKET` exported, or a stale id) has no home context, so an explicit `--scope context|window` fails with an error at the CLI and an unscoped notification escalates to `global` — reachable everywhere rather than attached to a context that never produced it.

Host-raised config errors are the one deliberate exception: they are workspace-wide rather than a property of any one context, so they are explicitly `global`.

## Agents

```toml
[agents]
os_notifications = true
low    = "claude --model claude-haiku-4-5 --dangerously-skip-permissions '{cmd}'"
medium = "claude --model claude-sonnet-4-6 --dangerously-skip-permissions '{cmd}'"
high   = "claude --dangerously-skip-permissions '{cmd}'"
restore_mode = "lazy"
```

`restore_mode` controls how persisted agent sessions (`SavedAgentSession` in `workspace.toml`) are restored on host startup:

- `"lazy"` (default): active-context agent panes with a `session_id` auto-resume in their restored `cwd` via their Harness Adapter (`claude --resume <id>`, `codex resume <id>`, `pi --session <id>`, `agy --resume <id>`), while background and sub-context agent panes restore their status dots, slots, and `Blocked` / `NeedsYou` badges into a lightweight `[Plexi Standby]` view. It resumes only when Enter is pressed or explicit input is sent with `plexi pane send`.
- `"eager"`: all persisted non-ephemeral agent panes with a `session_id` auto-resume immediately on startup.
- `"off"`: persisted agent panes restore as plain shell terminals without running harness resume commands.

In all modes, mid-turn `Working` panes restore as `Interrupted` with their `tail_preview`, and ephemeral `Completed` squad worker panes are skipped on startup. Set `[agents].standby_scrollback_lines` (default `20`) to control how many scrollback lines a Standby view saves and shows.

## CLI

```toml
[cli]
tips = true
```

Set `tips = false` to hide contextual tips after CLI commands.

## MCP servers

MCP servers live in their own file next to `config.toml` in the active channel
profile: `mcp_servers.toml`. It is user-owned — Plexi reads it and never writes
it — and it is the only source of a server's command line.

```toml
[servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/Users/you/projects"]

[servers.git]
command = "uvx"
args = ["mcp-server-git"]
env = { GIT_AUTHOR_NAME = "plexi" }
cwd = "/Users/you/projects/repo"
```

`command` is required; `args`, `env`, and `cwd` are optional, and an unknown key
is an error rather than a silent no-op. `cwd` defaults to the calling pane's
workspace root. A server child gets `PATH` and `HOME` plus whatever `env`
declares — nothing else from the host environment.

A server runs in its own process group and is killed — including anything an
`npx`/`uvx` launcher spawned — when the pane that opened it closes. A server
that never produces output, or never completes the MCP handshake, is
disconnected after a deadline with a visible error rather than left running.

An app never supplies a command: it names a server id and the host resolves the
rest here, so the `mcp.client` capability means "talk to a server you
configured", not "run this command". Servers reach the assistant through the
bridge app (`apps/dev/mcp-bridge`), which re-exposes their tools as ordinary
connector tools. Format and process contract: `src/host/mcp_client.rs`.

## Default config.toml

This is the built-in template Plexi writes when it creates or resets the active channel config.

```toml
# Plexi Configuration — full reference: https://plexiapp.com/docs/config | docs/CONFIG.md
# Edit: plexi config edit  |  Check: plexi config check  |  Reset: plexi config reset

config_version = 1

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
# sound = "/System/Library/Sounds/Ping.aiff"   # cue played on arrival; unset = silent

[ai]
# client = "personal"          # default AI ledger tag; a run can override it

# [models.fast]
# provider = "openrouter"
# model = "deepseek/deepseek-v4.1-flash"
# thinking = "low"
# [models.slow]
# provider = "openrouter"
# model = "openai/gpt-6-astra"
# thinking = "high"

# [log]
# level = "info"               # error | warn | info | debug — applies live on save, no restart
# retention_days = 30          # dated plexi-YYYY-MM-DD.log archives. The live plexi.log stays.

# [cloud]
# Local use never needs an account. Signing in only links this desktop to
# relay and cloud features. Unset, local ledger rows and assistant
# conversations are kept. Relay registry rows, queued envelopes, and the log
# archives above are still pruned.
# retain_local_history = false

# [keybindings]
# toggle_command_palette = "cmd+p"
# open_config            = "cmd+comma"
# close_context          = "cmd+shift+w"

[agents]
os_notifications = true
low    = "claude --model claude-haiku-4-5 --dangerously-skip-permissions '{cmd}'"
medium = "claude --model claude-sonnet-4-6 --dangerously-skip-permissions '{cmd}'"
high   = "claude --dangerously-skip-permissions '{cmd}'"
# restore_mode = "lazy"        # "lazy" (active context resumes, background enters Standby), "eager", or "off"

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
# login fails closed. An account is optional: local use never needs one. A
# session only links this desktop to relay and cloud features.
# account_backend = "plexi"
# Accounts service base URL. Unset = the official plexiapp.com service. Override
# only to point `plexi account login` at a private deployment.
# account_url     = "https://plexiapp.com"
# Default email pre-filled by `plexi account login`.
# account_email   = "you@example.com"
```
