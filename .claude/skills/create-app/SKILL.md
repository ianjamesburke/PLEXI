---
name: create-app
description: Scaffold a new Plexi app, install it, open it in a pane, and activate hot reload — all in one command. Use when the user says "build me an app", "create a Plexi app", or "make an app that does X".
skill_version: "3.5.89"
source: local
date_added: "2026-05-09"
---

# Create App

Scaffold a minimal working Plexi app, open it in a live pane, and enable hot reload so the agent can edit it immediately.

---

## Step 1 — Gather requirements

Ask (in a single message, not separately):

1. **App name** — a short slug (`snake_case`, no spaces). This becomes the app ID and directory name.
2. **One-sentence description** — what it does. Used in the manifest and as the initial render hint.

If the user already provided both (e.g. "make an app called `notes` that shows a text editor"), extract them and skip asking.

---

## Step 2 — Validate environment

```bash
# Must be inside a Plexi pane
if [ -z "$PLEXI_SOCKET" ]; then
  echo "error: PLEXI_SOCKET not set — run this inside a Plexi terminal pane"
  exit 1
fi
echo "PLEXI_SOCKET is set: $PLEXI_SOCKET"
```

```bash
# Must not be in home directory — workspace init from ~ collides with the global profile dir
if [ "$PWD" = "$HOME" ]; then
  echo "error: cannot create a Plexi app from the home directory"
  echo "Please cd into a project directory first"
  exit 1
fi

# Must have a workspace (CWD or ancestor has .plexi/)
plexi workspace init 2>/dev/null || true   # no-op if already initialised
```

---

## Step 3 — Scaffold

```bash
APP_NAME="<name>"
plexi app init "$APP_NAME"
```

This creates:
```
.plexi/apps/<name>/
  manifest.toml
  main.py
  plexi_sdk.py
```

---

## Step 4 — Patch manifest

Add `watch = true` to `[app]` so hot reload activates when the pane opens. Also inject the user's description.

Read the generated manifest:
```bash
cat ".plexi/apps/$APP_NAME/manifest.toml"
```

Edit it in-place — use the Read + Edit tools (not sed). The `[app]` section should become:

```toml
[app]
id = "<name>"
name = "<Display Name>"
type = "app"
entry = "main.py"
version = "0.1.0"
description = "<user's description>"
watch = true
```

Do NOT add any other fields or change the schema.

---

## Step 5 — Open the pane

```bash
PANE_ID=$(plexi open "$APP_NAME")
echo "Opened pane: $PANE_ID"
```

The host rescans the app registry on cache miss (v3.5.89+), so newly scaffolded apps are discoverable without restarting Plexi.

If `plexi open` prints nothing or exits non-zero:
1. Check `~/.plexi-alpha/plexi.log` (or the channel-appropriate log) for errors
2. Verify the manifest is valid: `plexi validate ".plexi/apps/$APP_NAME"`
3. Report the error — do not silently continue

---

## Step 6 — Report

Output exactly this block so the user can navigate immediately:

```
App created: <name>
Directory:   .plexi/apps/<name>/
Pane ID:     <PANE_ID>
Hot reload:  active (watch = true)

Edit main.py to update the app live. No restart needed.
```

---

## What the agent does next

After reporting, the agent is in the live-edit loop:

- Read `.plexi/apps/<name>/main.py`
- Edit it to implement what the user asked for
- The pane updates automatically on save (hot reload is active)
- Run `plexi validate ".plexi/apps/$APP_NAME"` if the pane goes blank (manifest issue)

---

## Skill constraints

- **One app per invocation** — if the user asks for multiple apps, create them sequentially, not in parallel (each `plexi open` returns a pane ID that must be captured before the next)
- **Never run `plexi workspace init` from `~`** — check `pwd` first
- **`plexi open` must be run from inside a Plexi pane** — `PLEXI_SOCKET` must be set, or the call queues to the spawn-queue without returning a pane ID
- **The app ID must be unique** — if `.plexi/apps/<name>/` already exists, `plexi app init` will exit with an error; surface it to the user and ask for a different name
- **Do not modify `plexi_sdk.py`** — it is generated from the host's embedded SDK; editing it will be overwritten on the next install
