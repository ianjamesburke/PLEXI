---
name: plexi-cli
description: Operating inside Plexi — spawn/name panes, focus, launch apps, surface notifications. Use when working in a Plexi pane or orchestrating other panes.
skill_version: "3.8.0"
plexi_version: "0.0.638"
---

# Plexi CLI

You are running inside a Plexi pane. `PLEXI_SOCKET` is set automatically — every `plexi` command routes to the correct running instance.

**Before using any subcommand**, run `plexi <noun> --help` to confirm it exists and check its flags. Subcommands change across releases — never assume.

## Non-Obvious Translation Rules

- `plexi pane new` is primary; `plexi terminal` is a legacy alias — prefer `pane new`
- `plexi app open <type-id>` opens installed apps — never `plexi pane send <id> "app\n"`
- `plexi app render` takes an installed app **ID** (not a path) — run `plexi app list` first
- `plexi agent init` replaces the former `plexi app init --agent` form
- `plexi pane command <id> "text" --enter` = send + newline in one step (terminal panes only)

## Footguns

- **Never** run `plexi workspace init` from `~` — it collides with the profile dir at `~/.plexi/`
- `plexi pane send <id> "text\n"` submits in **shell** panes but does **not** submit Claude Code prompts — use the two-step below
- `plexi pane state` returns the L1 UiNode tree for app panes; it returns nothing useful for terminal panes

## Env Vars (set automatically in every pane)

- `PLEXI_SOCKET` — IPC socket path; routes all `plexi` commands to the running instance
- `PLEXI_PANE_ID` — numeric ID of the current pane; pass to `--from-pane-id` without a round-trip
- `PLEXI_CONTEXT_ID` / `PLEXI_CONTEXT_NAME` — context the pane belongs to

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

### Fire-and-forget notification (background — use only when pane is about to close)

```bash
(RESULT=$(plexi notify --title "Done" --body "Branch pushed." --choice "ok:Dismiss")
 [ "$RESULT" = "open" ] && open "<pr-url>") &
```

### 2×2 grid layout (apps in three quadrants)

```bash
BALLS=$(plexi app open balls -r --from-pane-id $PLEXI_PANE_ID)
plexi app open tetris -d --from-pane-id $PLEXI_PANE_ID
plexi app open snake -d --from-pane-id $BALLS
```
