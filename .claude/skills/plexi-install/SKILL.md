---
name: plexi-install
description: "Install a Plexi app by scaffolding or copying it into ~/.plexi/apps/. Knows the manifest.toml format, SDK conventions, and capability fields."
risk: low
source: local
date_added: "2026-04-11"
---

# Plexi Install

Installs or scaffolds a Plexi app into `~/.plexi/apps/`.

## When to invoke

User says: `/plexi-install`, "install this as a plexi app", "add [thing] to plexi", "scaffold a new plexi app called [name]"

## App structure

Every installed app lives at `~/.plexi/apps/<app-id>/` and contains:

```
~/.plexi/apps/<app-id>/
  manifest.toml      # required — app identity + capabilities
  <entry>.py         # required — main app script
  plexi_sdk.py       # required — copy from any existing app
```

### manifest.toml format

```toml
[app]
id = "my-app"             # kebab-case, matches directory name
name = "My App"           # display name shown in Plexi UI
entry = "my_app.py"       # filename of the main script
version = "0.1.0"
description = "One sentence."

[app.capabilities]
file_types = []           # file extensions this app handles, e.g. ["md", "txt"]
terminal_write = false    # true if app needs to write to the terminal
filesystem = "none"       # "none" | "read" | "read_write"
```

### plexi_sdk.py

Copy from an existing app — it's the same file in all apps:
```bash
cp ~/.plexi/apps/wikipedia/plexi_sdk.py ~/.plexi/apps/<app-id>/
```

## What to do

**Case A — user has an existing script to install:**
1. Determine app-id from the script name or user's description (kebab-case)
2. Create `~/.plexi/apps/<app-id>/`
3. Copy the script there
4. Copy `plexi_sdk.py` from `~/.plexi/apps/wikipedia/`
5. Write `manifest.toml` based on what the script does
6. Confirm: "Installed as `<app-id>`. Restart Plexi to load it."

**Case B — user wants to scaffold a new app from scratch:**
1. Ask: what should it do? (if not already described)
2. Create the directory and `manifest.toml`
3. Write a minimal Python scaffold that imports `plexi_sdk` and renders a placeholder UI
4. Copy `plexi_sdk.py`
5. Tell the user: "Scaffolded at `~/.plexi/apps/<app-id>/`. Edit `<entry>.py` to build it out."

**Case C — user wants to install from a project directory:**
1. Check if the directory has a `manifest.toml` already
2. If yes — copy the whole directory to `~/.plexi/apps/<app-id>/`, copy SDK if missing
3. If no — infer the manifest from the directory contents and write one

## After installing

Always end with:
> App installed at `~/.plexi/apps/<app-id>/`. Restart Plexi to pick it up (hot-reload is not yet supported for new apps).

## Notes

- `app.id` must match the directory name exactly.
- `filesystem = "none"` is the safe default — only escalate if the app actually needs disk access.
- `terminal_write = false` is correct for read-only/display apps; set `true` only if the app injects text into the terminal.
- The SDK file is identical across all apps — always copy it, never modify it.
