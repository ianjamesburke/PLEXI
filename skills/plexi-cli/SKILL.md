---
name: plexi-cli
description: Operating inside Plexi — spawn/name panes, focus, launch apps, surface notifications. Use when working in a Plexi pane or orchestrating other panes.
skill_version: "3.6.58"
---

# Plexi CLI

You are running inside a Plexi pane. `PLEXI_SOCKET` is set automatically — every `plexi` command routes to the correct running instance.

## Panes

```bash
plexi pane self                        # print current pane ID (just the number, no JSON)
plexi pane list                        # JSON array: [{id, title, type, context_id, focused}]
plexi pane info                        # JSON info for current pane (uses PLEXI_PANE_ID)
plexi pane name "title"                # rename current pane
plexi pane name <pane_id> "title"      # rename any pane by ID
plexi pane focus <pane_id>             # move UI focus to a pane
plexi pane close [pane_id]             # close pane (omit = current via PLEXI_PANE_ID)
plexi pane send <pane_id> "text"       # inject text to a pane's PTY (text only, no newlines)
plexi pane key <pane_id> <key>         # send keystroke: "enter", "escape", "ctrl+c", etc.
plexi pane capture [pane_id] --lines N # read last N lines of scrollback as JSON array (default 50)
```

`plexi terminal [cmd]` opens a terminal pane, returns pane ID on stdout. Flags: `-e/--ephemeral` (close the pane when the command process exits), `--layout <LAYOUT>`, `--from-pane-id <id>`, `--cwd <path>`, `--no-focus`.

By default (no `--ephemeral`), the pane will remain open after your command finishes—so if you’re used to shells like **zsh** staying interactive, you can assume that “the terminal stays around” unless you explicitly opt into `-e/--ephemeral`. 

**Layout values:** `split_h` (right), `split_right` (alias for split_h), `split_v` (below), `split_below` (alias for split_v), `split_above`, `tab` (new tab in current window), `new_window` (new OS window).

```bash
LANE=$(plexi terminal --layout split_h)                 # pane to the right
QUEUE=$(plexi terminal --layout tab --from-pane-id $LANE)  # tab in LANE's window
NEW=$(plexi terminal --layout new_window)                # separate OS window
```

`--from-pane-id` requires `PLEXI_SOCKET`. Use `$PLEXI_PANE_ID` (set automatically in every pane) — no extra round-trip needed. Only call `plexi pane self` if you need to capture the ID into a variable for reuse.

## Apps

`plexi open <app-id>` launches an installed app. Flags: `--layout <left|right|top|bottom|overlay|split_h|split_v|split_right|split_below>`, `--from-pane-id <id>` (split relative to a pane), `--mcp`, `--cli`.

```bash
plexi list                       # list installed apps
plexi app init                   # scaffold a new app in cwd
plexi install <path>             # install from local dir
plexi uninstall <app-id>         # uninstall
plexi update                     # update apps or self
plexi validate [path]            # validate app dir
```

**Always `plexi open` for installed apps** — never `plexi pane send <id> "app\n"`. Never create terminal panes as placeholders; `plexi open` supports `--from-pane-id` and `--layout` directly.

2×2 grid example (apps in 3 quadrants):
```bash
BALLS=$(plexi open balls --layout split_h --from-pane-id $PLEXI_PANE_ID)
plexi open tetris --layout split_v --from-pane-id $PLEXI_PANE_ID
plexi open snake --layout split_v --from-pane-id $BALLS
```

## Notifications

Requires `PLEXI_SOCKET`. Flags: `--level info|warn|error`, `--timeout <seconds>` (0 = no timeout), `--scope <window|context|global>` (default: global).

Choices via `--choice "key:Label"` (returns key). For host-side actions, add `--host-action "key:pane_focus:$PANE"`. Snooze: `--choice "snooze:300:Remind me in 5m"`. Labels: 3–5 words, verb + object.

**Blocking** (decision gate — cycle waits for user):
```bash
RESULT=$(plexi notify --title "PR ready" --body "Summary." \
  --choice "a:Open PR" --choice "b:Skip" --choice "c:Talk to Claude" \
  --host-action "c:pane_focus:$MY_PANE")
case "$RESULT" in
  a) open "<pr-url>" ;;
  b) ;; # skip
  c) ;; # pane_focus handled host-side
esac
```

**Fire-and-forget** (end-of-run only — background subshell, pane closes):
```bash
(RESULT=$(plexi notify --title "Shipped" --body "PR merged" \
  --choice "ok:Dismiss" --choice "open:Open PR")
 [ "$RESULT" = "open" ] && open "<pr-url>") &
```

## Workspaces & Contexts

```bash
plexi workspace init             # initialise a .plexi/ workspace in the current dir
                                 # ⚠ never run from ~ — collides with profile dir

plexi context new [path]         # create a new context, optionally at a path
plexi context open <path>        # open a context at a path
plexi context set-root <path>    # set the root directory for the active context
plexi context current            # print context ID and name as JSON (reads env vars set at pane spawn)
```

Every terminal pane has two env vars set at spawn time:
- `PLEXI_CONTEXT_ID` — numeric ID of the context the pane belongs to
- `PLEXI_CONTEXT_NAME` — display name of that context

`plexi context current` reads these and prints `{"context_id": N, "context_name": "..."}`. Returns exit 1 if run outside a Plexi pane.

## Secrets

```bash
plexi secret set <NAME>          # prompts for value; scoped to nearest .plexi/ workspace
plexi secret set <NAME> --global # store cross-workspace (global fallback)
plexi secret set <NAME> --from-env  # read value from env var NAME instead of prompting
plexi secret list                # list stored secrets
plexi secret delete <NAME>       # delete a secret
```

## Commands

```bash
plexi run <command>              # run a named command from .plexi/commands.toml
```

## Pack & Registry

```bash
plexi pack export                # export current apps as a pack file
plexi registry watch             # watch installed CLIs for descriptor drift
```

## Orchestration Pattern (parallel ship sessions)

```bash
# Spawn one worker per issue in separate windows:
LANE1=$(plexi terminal "cl '/ship-issue 807'" --layout new_window)
LANE2=$(plexi terminal "cl '/ship-issue 810'" --layout new_window)

# Queue next issues as tabs within each lane window:
plexi pane focus $LANE1
plexi terminal "cl '/ship-issue 812'" --layout tab --no-focus

# Name panes and notify on completion:
plexi pane name $LANE1 "Lane 1: #807"
plexi notify \
  --title "PR #807 ready for review" \
  --choice "Go to pane:pane_focus:$LANE1"
```

For full dispatch orchestration (auto-computed lanes from issue state), see the `/dispatch-next` skill.

## Cross-Pane Conversations

Send messages to and read responses from coding assistants (Claude Code, Codex, Hermes, etc.) running in other panes. Assistant-agnostic — relies only on scrollback behavior, not UI-specific markers.

### Sending a message

`\n` in `pane send` does NOT produce Enter — it types a literal backslash-n. Always send text and Enter separately:

```bash
plexi pane send <pane_id> "your message here" && plexi pane key <pane_id> enter
```

### Waiting for a response

The universal idle signal: **scrollback stopped changing.** Two identical captures with a gap = done.

```bash
TARGET=<pane_id>
PREV=""
until [ -n "$PREV" ] && [ "$PREV" = "$(plexi pane capture $TARGET --lines 3)" ]; do
  PREV=$(plexi pane capture $TARGET --lines 3)
  sleep 3
done
```

### Reading the response

Capture the last N lines of scrollback. **Start with `--lines 80`.** If the response is clearly truncated (mid-sentence, no closing), step up to `--lines 150` once. Never go higher — if 150 isn't enough, the response is too long to pull into your context anyway; summarize what you have.

```bash
plexi pane capture <pane_id> --lines 80
```

To extract only content after the message you sent:

```bash
plexi pane capture <pane_id> --lines 80 | \
  sed -n '/your message/,$ p' | tail -n +2
```

### Full send-wait-read pattern

```bash
TARGET=<pane_id>
MSG="your question here"

# 1. Send
plexi pane send $TARGET "$MSG" && plexi pane key $TARGET enter

# 2. Wait for idle (scrollback stabilizes)
PREV=""
until [ -n "$PREV" ] && [ "$PREV" = "$(plexi pane capture $TARGET --lines 3)" ]; do
  PREV=$(plexi pane capture $TARGET --lines 3)
  sleep 3
done

# 3. Read response
plexi pane capture $TARGET --lines 80
```
