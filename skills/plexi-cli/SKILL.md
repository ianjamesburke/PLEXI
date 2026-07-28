---
name: plexi-cli
description: Operating inside Plexi — spawn/name panes, focus, launch apps, manage contexts, surface notifications. Use when working in a Plexi pane or orchestrating other panes.
skill_version: "4.4.0"
plexi_version: "0.2.0"
last_verified: "2026-07-28"
---

# Plexi CLI

You are running inside a Plexi pane. `PLEXI_SOCKET` is set automatically -- every `plexi` command routes to the correct running instance.

**Before using any subcommand**, run `plexi <noun> --help` to confirm it exists and check its flags. Subcommands change across releases -- never assume.

## Env Vars (set automatically in every pane)

| Var | Purpose |
|-----|---------|
| `PLEXI_SOCKET` | IPC socket path; routes all commands to the running instance |
| `PLEXI_PANE_ID` | Numeric ID of the current pane; pass to `--from` |
| `PLEXI_CONTEXT_ID` | Context ID the pane belongs to |
| `PLEXI_CONTEXT_NAME` | Context name the pane belongs to |

## Command Reference

### pane -- create, control, and inspect panes

```
pane new [CMD]           Open a terminal pane. Flags: -n NAME, -d/--down, -l/--left, -u/--up,
                         -r/--right, --tab, --window, --overlay, --from PANE_ID, -e/--ephemeral,
                         --no-focus, --cwd DIR
pane new --agent ALIAS   Spawn a pane, launch the agent alias, and BLOCK until the agent TUI
                         reports a booted idle prompt; prints the pane id on stdout only then.
                         --boot-timeout SECS (default 60). Exit 0 = id on stdout, ready for a
                         brief. Exit 2 = boot timeout (created pane id on stderr — close it).
                         Exit 1 = usage/plumbing. Combines with the placement flags above.
pane name [ID] NAME      Rename. One arg = rename self. Two args = rename by ID.
pane list [--context [ID]]  JSON array of panes. --context (no arg) = caller's context.
pane focus ID            Move visible focus to a pane (does NOT move the agent).
pane close [ID]          Close a pane. Omit ID = close self.
pane send ID TEXT        Type text into a pane. Use \n for Enter.
pane send ID TEXT -s     --submit/-s: host types the text, waits for the input line to settle,
                         presses Enter, confirms submission, and self-heals a collapsed paste
                         with exactly one internal retry. Exit 0 = confirmed submitted; exit 1 =
                         typed but unconfirmed (observed input line on stderr). Terminal panes only.
pane key ID KEY          Send a keypress. Named: enter, escape, space, up, down, left, right,
                         backspace. Chords: ctrl+c.
pane command ID TEXT -e  Send text + Enter in one step (terminal panes only).
pane capture [ID]        Last N lines as JSON. --lines N (default 50), --from-cursor CURSOR,
                         --full-output. --plain prints raw lines only on stdout and the next
                         cursor on stderr; --plain and --from-cursor compose.
pane status ID           One host-derived JSON verdict: working, idle, blocked, or unknown,
                         with high/low confidence and raw agent/status-bar/buffer evidence.
pane self                Print current pane's ID.
pane info                Print current pane details as JSON.
pane state ID            App panes: L1 UiNode tree JSON. Terminal panes: simple status object.
pane slot write NAME [CONTENT]  Write a named per-pane byte slot (stdin when CONTENT omitted).
                         --pane-id ID (default: PLEXI_PANE_ID), --append or --replace for an
                         existing slot.
pane slot read NAME [PANE_ID]   Print a slot's raw bytes.
pane slot wait NAME [PANE_ID]   Block until the slot's value matches --until REGEX; prints the
                         matching value. --timeout SECS (default 300). Level-triggered: an
                         already-matching value returns immediately. Exit 0 = match, exit 2 =
                         timeout (nothing on stdout), exit 1 = usage/plumbing — branch on the
                         exit code alone.
pane slot list [PANE_ID]        List a pane's slots as JSON.
pane slot delete NAME [PANE_ID] Delete a slot.
```

### app -- install, open, scaffold, inspect

```
app open [TYPE_ID]       Open an installed app. Flags: -d, -l, -u, -r, --tab, --window,
                         --from PANE_ID. Extra args forwarded to the app.
app open --mcp CMD...    Wrap a stdio MCP server in a Plexi pane.
app open --cli BINARY    Wrap a CLI tool with a Plexi UI.
app init NAME            Scaffold a new app. --lang python (default), --global, --no-open,
                         --from PANE_ID. Before building a canvas/on_render app, read
                         the SDK reference at https://plexiapp.com/docs/sdk for the
                         drawing API (rect stroke/glow/gradient, circle glow, arc_ring,
                         theme tokens).
app install [SPEC]       Install from path, github:owner/repo, or --pack core.
                         No args = install from .plexi/apps.toml. --version SEMVER to pin.
app uninstall ID         Remove an installed app.
app list                 Show all installed apps with versions.
app info ID              Show app details: id, name, version, tools.
app render APP           Render an app (installed id or local path, e.g. ".") without a
                         running host. Default output is the JSON UI tree; --png for an
                         image. --size WxH, --state FILE, --output FILE.
app validate [PATH]      Check app dir for errors before publish/install.
app freeze PATH          Export installed apps as TOML snapshot (like pip freeze).
app publish              Stub for future marketplace publishing; confirm behavior with --help.
app update               Check for available updates.
app action ID ACTION [ARGS]  Send a semantic action to a running app (not raw text).
```

### context -- manage project scopes

```
context new [NAME]       New context. --path DIR, --parent[=NAME] (bare = current context), --from PANE_ID (portal anchor; defaults to caller), -d/-l/-u/-r portal direction, --focus, --window CMD (repeatable; creates extra windows atomically).
context sub NAME         Sub-context under the caller's context, pre-populated with panes.
                         --agents N (1-32, default 1), --command CMD (once = all panes, or exactly N times = one each; any other count errors),
                         --layout tiled|columns (default tiled), --path DIR (defaults to caller's cwd), --focus, --from PANE_ID.
                         Prints {"context_id","windows","panes":[ids]} — address the panes directly, no follow-up `pane list`.
context open [PATH]      Switch current pane to a context at PATH.
context set-root [PATH]  Change root folder for the caller's context (PLEXI_CONTEXT_ID).
context current          Print context id/name as JSON.
context describe TEXT    Set description for the caller's context (PLEXI_CONTEXT_ID).
context zoom ID          Zoom into a sub-context.
context zoom-out         Zoom out to parent context.
context push [NAME]      Push a pane into a new sub-context. --pane-id ID (defaults to the calling pane).
```

### notify -- surface information to the user

```
notify --title TEXT      Required. --body TEXT, --level info|warn|error,
                         --timeout SECS (0=sticky), --scope window|context|global
                         (default context — stays in the sending context;
                         pass --scope global to surface everywhere).
                         The sending context is the caller's own (PLEXI_CONTEXT_ID),
                         never the currently active one. Outside a pane there is no
                         sending context: --scope context|window errors, and an
                         unscoped notify escalates to global.
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
agent init NAME          Scaffold agent app (ai.query + chat UI). --from PANE_ID.
agent add NAME           Install from global registry into workspace.
agent update NAME        Re-install from registry, preserving memory/logs.
agent list               List workspace agents.
agent report             Hook-only state report. --state working|blocked|idle, --agent NAME,
                         --session-id ID, --detail TEXT.
agent status             Table of pane agent state. Adds DETAIL when any pane reports it.
```

A reported agent is addressable, not just observable: every pane with agent state
becomes a row in the human's Cmd+P palette, across every context and window, keyed
on the `--agent` name and annotated with `--detail`. Same-name agents are numbered
within their context; blocked agents sort first. Report a name a human would
search for; keep `--detail` to the active tool.

### workspace, run, routine

```
workspace init           Set up .plexi/ in current dir. NEVER run from ~.
run [COMMAND]            Run a named command from .plexi/commands.toml. Omit to list.
routine list             Show the workspace's routines with schedule/next fire. Resolves routines.toml from the workspace root (works from any subdirectory).
routine run NAME         Manually trigger a routine. Fires into its configured context like a scheduled run; --force fires a disabled routine.
routine add NAME --command CMD --schedule SPEC [--context CTX] [--ephemeral]
                         Append a routine. Schedule is validated by the scheduler's own parser; comments in the file are preserved.
routine remove NAME      Delete a routine from routines.toml.
routine enable NAME      Re-enable a disabled routine (removes enabled = false).
routine disable NAME     Keep the routine but never fire it (sets enabled = false).
```

### ai, config, doctor, notes, demo, update, uninstall

```
ai doctor [--json]       Scan hardware, recommend models, check Ollama/OpenRouter.
ai setup                 Interactive wizard to configure local AI via Ollama.
config check             Validate config.toml.
config edit              Open config.toml in $EDITOR.
config get KEY           Print resolved value (e.g. agents.medium).
config list              Print all known keys with type, value, description. --json for machine output.
config set KEY=VAL ...   Set one or more keys in-place (e.g. config set theme.preset=dracula font_size=14).
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

## Config Workflow for Agents

Before writing any config key, run `plexi config list` to discover valid keys, their types, and current values. Then use `plexi config set KEY=VALUE` to apply changes without opening an editor. Scope: workspace if inside a workspace, global otherwise (override with -g/--global or -w/--workspace).

## Non-Obvious Translation Rules

- `pane new` is the canonical way to open a terminal pane
- `app open TYPE_ID` opens installed apps -- never `pane send ID "app\n"`
- `app render . --png` renders the app in the current dir with no running host; default output is JSON, not an image
- `agent init` replaces the former `app init --agent` form
- `pane command ID "text" --enter` = send + Enter in one step (terminal panes only)
- `pane send` with `\n` submits in shell panes but does NOT submit Claude Code prompts -- use `pane send ID TEXT --submit` (or the legacy two-step pattern below on builds without it)
- `app action` delivers structured semantic events; `pane command` sends raw keystrokes

## Footguns

- **Never** run `workspace init` from `~` -- collides with profile dir at `~/.plexi/`
- `pane send ID "text\n"` submits in **shell** panes but does **not** submit Claude Code prompts -- `--submit` is the sanctioned path into an agent TUI
- `pane slot wait` exits 2 on timeout and 0 on match -- branch on the exit code, never parse stderr
- `pane capture --plain` reserves stdout for captured text; read `cursor=N` from stderr when advancing a delta loop
- `pane status ID` is the single-call corroboration path when a pane slot is missing or stale; `unknown`/`low` means escalate instead of guessing
- `pane state` returns L1 UiNode tree for app panes; returns a simple status object for terminal panes
- `pane focus` moves what the user **sees**, not where the agent runs -- the agent stays in its own pane
- `app open --mcp` and `--cli` are mutually exclusive with TYPE_ID
- `notify` without `--choice` is fire-and-forget; with `--choice` it **blocks** until the user picks

## Multi-Command Patterns

### Send a message to an agent (Claude Code / Codex) pane

```bash
plexi pane send $TARGET "your message here" --submit
```

Exit 0 means the host observed the prompt leave the input line; non-zero means typed but unconfirmed (the observed input line is on stderr) -- do not blind-retry. Legacy two-step for builds without `--submit`:

```bash
plexi pane send $TARGET "your message here"
plexi pane key $TARGET enter
```

### Check whether an agent pane is idle

```bash
plexi pane status "$TARGET"
```

Trust `idle` only with `high` confidence. Treat `unknown` or `low` confidence as an escalation signal, not permission to guess from repeated captures.

### Cursor-based incremental capture

```bash
CURSOR_FILE=$(mktemp)
plexi pane capture "$TARGET" --lines 1 --plain 2>"$CURSOR_FILE"
CURSOR=$(sed -n 's/^cursor=//p' "$CURSOR_FILE")
# ... wait for new output ...
plexi pane capture "$TARGET" --from-cursor "$CURSOR" --plain 2>"$CURSOR_FILE"
```

Captured text is stdout. The next `cursor=N` marker is stderr; strip the prefix before passing it to the next `--from-cursor`.

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
BALLS=$(plexi app open balls -r --from $PLEXI_PANE_ID)
plexi app open tetris -d --from $PLEXI_PANE_ID
plexi app open snake -d --from $BALLS
```

### Scoped agent squad (one command)

```bash
# 3 agents in their own sub-context, rooted at the current cwd.
SQUAD=$(plexi context sub agentsquad --agents 3 --command claude)
# Pane ids come back in the response — no follow-up `pane list` needed.
for P in $(echo "$SQUAD" | jq -r '.panes[]'); do plexi pane name $P "squad-$P"; done
```

Prefer this over `context new --parent --window CMD ...`: that route seeds a
spare terminal (N commands → N+1 panes), spreads the panes across sibling
*pages* instead of tiling them in one window, and roots the sub-context at the
parent context's path rather than your cwd.

### Named agent dispatch lane

```bash
LANE=$(plexi pane new -n "issue-42" -r --from $PLEXI_PANE_ID --agent claude) || exit 1
plexi pane send $LANE "investigate and fix issue 42" --submit
```

`--agent` prints the id only once the agent is booted and ready for a brief, so the send cannot race the TUI boot. Legacy form for builds without these flags:

```bash
LANE=$(plexi pane new -n "issue-42" -r --from $PLEXI_PANE_ID)
plexi pane send $LANE "investigate and fix issue 42"
plexi pane key $LANE enter
```

### Block on another pane's progress slot

```bash
if VERDICT=$(plexi pane slot wait verdict $LANE --until 'PASS|FAIL' --timeout 600); then
  echo "tester says: $VERDICT"
elif [ $? -eq 2 ]; then
  echo "no verdict within 10 min -- escalate" >&2
fi
```

### Semantic app action (no keystroke simulation)

```bash
plexi app action $PANE_ID refresh
plexi app action $PANE_ID navigate-to /some/path
```
