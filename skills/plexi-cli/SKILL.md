---
name: plexi-cli
description: Operating inside Plexi — spawn/name panes, focus, launch apps, manage contexts, surface notifications. Use when working in a Plexi pane or orchestrating other panes.
skill_version: "4.0.0"
plexi_version: "0.0.651"
last_verified: "2026-06-07"
---

# Plexi CLI

You are running inside a Plexi pane. `PLEXI_SOCKET` is set automatically -- every `plexi` command routes to the correct running instance.

**Before using any subcommand**, run `plexi <noun> --help` to confirm it exists and check its flags. Subcommands change across releases -- never assume.

## Env Vars (set automatically in every pane)

| Var | Purpose |
|-----|---------|
| `PLEXI_SOCKET` | IPC socket path; routes all commands to the running instance |
| `PLEXI_PANE_ID` | Numeric ID of the current pane; pass to `--from` / `--from-pane-id` |
| `PLEXI_CONTEXT_ID` | Context ID the pane belongs to |
| `PLEXI_CONTEXT_NAME` | Context name the pane belongs to |

## Command Reference

### pane -- create, control, and inspect panes

```
pane new [CMD]           Open a terminal pane. Flags: -n NAME, -d/--down, -l/--left, -u/--up,
                         -r/--right, --tab, --window, --overlay, --from PANE_ID, -e/--ephemeral,
                         --no-focus, --cwd DIR
pane name [ID] NAME      Rename. One arg = rename self. Two args = rename by ID.
pane list [--context [ID]]  JSON array of panes. --context (no arg) = caller's context.
pane focus ID            Move visible focus to a pane (does NOT move the agent).
pane close [ID]          Close a pane. Omit ID = close self.
pane send ID TEXT        Type text into a pane. Use \n for Enter.
pane key ID KEY          Send a keypress. Named: enter, escape, space, up, down, left, right,
                         backspace. Chords: ctrl+c.
pane command ID TEXT -e  Send text + Enter in one step (terminal panes only).
pane capture [ID]        Last N lines as JSON. --lines N (default 50), --from-cursor CURSOR,
                         --full-output.
pane self                Print current pane's ID.
pane info                Print current pane details as JSON.
pane state ID            App panes: L1 UiNode tree JSON. Terminal panes: simple status object.
```

### app -- install, open, scaffold, inspect

```
app open [TYPE_ID]       Open an installed app. Flags: -d, -l, -u, -r, --tab, --window,
                         --from-pane-id ID. Extra args forwarded to the app.
app open --mcp CMD...    Wrap a stdio MCP server in a Plexi pane.
app open --cli BINARY    Wrap a CLI tool with a Plexi UI.
app init NAME            Scaffold a new app. --lang python (default), --global, --no-open,
                         --from-pane-id ID.
app install [SPEC]       Install from path, github:owner/repo, or --pack core.
                         No args = install from .plexi/apps.toml. --version SEMVER to pin.
app uninstall ID         Remove an installed app.
app list                 Show all installed apps with versions.
app info ID              Show app details: id, name, version, tools.
app render ID            Render to PNG. --size WxH, --state FILE, --output FILE.
app validate [PATH]      Check app dir for errors before publish/install.
app freeze PATH          Export installed apps as TOML snapshot (like pip freeze).
app publish              Publish to Plexi marketplace.
app update               Check for available updates.
app action ID ACTION [ARGS]  Send a semantic action to a running app (not raw text).
```

### context -- manage project scopes

```
context new [NAME]       New context. --path DIR, --parent NAME.
context open [PATH]      Switch current pane to a context at PATH.
context set-root [PATH]  Change root folder for active context.
context current          Print context id/name as JSON.
context describe TEXT    Set description for active context.
context zoom ID          Zoom into a sub-context.
context zoom-out         Zoom out to parent context.
context push [NAME]      Push focused pane into a new sub-context.
```

### terminal -- shorthand for pane new

```
terminal [CMD]           Same as pane new. Flags: -e, --layout LAYOUT, --from-pane-id ID,
                         --cwd DIR, --no-focus.
```

### notify -- surface information to the user

```
notify --title TEXT      Required. --body TEXT, --level info|warn|error,
                         --timeout SECS (0=sticky), --scope window|context|global.
                         --choice "key:Label" (repeatable, blocks until chosen).
                         --host-action "key:action_type:arg" (runs even after caller exits).
```

### secret -- keychain-backed project secrets

```
secret set NAME          Prompts for value. --from-env, --global, --alias NAME.
secret get NAME          Print value. --global.
secret list              Show all secrets for current project.
secret delete NAME       Remove a secret.
```

### agent -- workspace agent definitions

```
agent init NAME          Scaffold agent app (ai.query + chat UI). --from-pane-id ID.
agent add NAME           Install from global registry into workspace.
agent update NAME        Re-install from registry, preserving memory/logs.
agent list               List workspace agents.
```

### workspace, run, routine

```
workspace init           Set up .plexi/ in current dir. NEVER run from ~.
run [COMMAND]            Run a named command from .plexi/commands.toml. Omit to list.
routine list             Show routines from .plexi/routines.toml with schedule/next fire.
routine run NAME         Manually trigger a routine.
```

### ai, config, doctor, notes, demo, update, uninstall

```
ai doctor [--json]       Scan hardware, recommend models, check Ollama/OpenRouter.
ai setup                 Interactive wizard to configure local AI via Ollama.
config check             Validate config.toml.
config edit              Open config.toml in $EDITOR.
config get KEY           Print resolved value (e.g. agents.medium).
config reset             Overwrite config.toml with defaults.
doctor [--json]          Audit installed apps for capability/config gaps.
notes list               Print scratchpad note paths, newest first.
notes open               Open note picker with fzf.
demo                     Interactive keybinding tutorial (split, navigate).
update                   Update Plexi binary.
update apps              Update installed apps.
completions SHELL        Print completion script (zsh, bash, fish).
uninstall                Remove app bundle, CLI, completions. --keep-data, -y.
```

## Non-Obvious Translation Rules

- `pane new` is primary; `terminal` is a shorthand alias -- prefer `pane new`
- `app open TYPE_ID` opens installed apps -- never `pane send ID "app\n"`
- `app render` takes an installed app **ID** (not a path) -- run `app list` first
- `agent init` replaces the former `app init --agent` form
- `pane command ID "text" --enter` = send + Enter in one step (terminal panes only)
- `pane send` with `\n` submits in shell panes but does NOT submit Claude Code prompts -- use the two-step pattern below
- `app action` delivers structured semantic events; `pane command` sends raw keystrokes

## Footguns

- **Never** run `workspace init` from `~` -- collides with profile dir at `~/.plexi/`
- `pane send ID "text\n"` submits in **shell** panes but does **not** submit Claude Code prompts
- `pane state` returns L1 UiNode tree for app panes; returns a simple status object for terminal panes
- `pane focus` moves what the user **sees**, not where the agent runs -- the agent stays in its own pane
- `app open --mcp` and `--cli` are mutually exclusive with TYPE_ID
- `notify` without `--choice` is fire-and-forget; with `--choice` it **blocks** until the user picks

## Multi-Command Patterns

### Send a message to a Claude Code pane (two steps required)

```bash
plexi pane send $TARGET "your message here"
plexi pane key $TARGET enter
```

### Wait for idle (scrollback stops changing)

```bash
PREV=""
until [ -n "$PREV" ] && [ "$PREV" = "$(plexi pane capture $TARGET --lines 3)" ]; do
  PREV=$(plexi pane capture $TARGET --lines 3)
  sleep 3
done
plexi pane capture $TARGET --lines 80
```

Start capture at `--lines 80`. Step up to `--lines 150` only if the response is clearly truncated. Never go higher.

### Cursor-based incremental capture

```bash
FIRST=$(plexi pane capture $TARGET --lines 1)
# ... wait for new output ...
plexi pane capture $TARGET --from-cursor "$CURSOR_FROM_PREVIOUS"
```

### Blocking notification (waits for user choice)

```bash
RESULT=$(plexi notify --title "PR ready" --body "Review and approve." \
  --choice "a:Open PR" --choice "b:Skip" --choice "c:Talk to Claude" \
  --host-action "c:pane_focus:$MY_PANE")
case "$RESULT" in
  a) open "<pr-url>" ;;
  b) ;;
  c) ;;  # pane_focus handled host-side
esac
```

### Fire-and-forget notification (background -- use only when pane is about to close)

```bash
(RESULT=$(plexi notify --title "Done" --body "Branch pushed." --choice "ok:Dismiss")
 [ "$RESULT" = "open" ] && open "<pr-url>") &
```

### 2x2 grid layout (apps in three quadrants)

```bash
BALLS=$(plexi app open balls -r --from-pane-id $PLEXI_PANE_ID)
plexi app open tetris -d --from-pane-id $PLEXI_PANE_ID
plexi app open snake -d --from-pane-id $BALLS
```

### Named agent dispatch lane

```bash
LANE=$(plexi pane new -n "issue-42" -r --from $PLEXI_PANE_ID)
plexi pane send $LANE "/implement-issue 42"
plexi pane key $LANE enter
```

### Semantic app action (no keystroke simulation)

```bash
plexi app action $PANE_ID refresh
plexi app action $PANE_ID navigate-to /some/path
```
