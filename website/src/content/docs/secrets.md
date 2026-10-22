---
title: Secrets
description: Store and access secrets inside Plexi apps.
order: 6
---

Plexi has a built-in secrets store scoped to each workspace. On macOS, secrets
are stored through the system keychain. On Linux, global, workspace, and
folder secrets share the backend described under [Linux storage](#linux-storage).
Secrets are available to apps through the host-brokered `secrets.get`
capability.

## Set a Secret

Open **Secrets** with Cmd+Shift+S or choose **Open Secrets** in the command
palette. The app lists global and current-workspace secrets. Folder secrets
appear when the `folder_secrets` experiment is on.

Use `j`/`k` to select, `Enter` to replace a value, `a` to add, and `d` to
delete. Deletion asks for confirmation; press `d` or `y` to confirm, or Esc
to cancel. Values stay masked, and editing starts with an empty replacement
field.

Each row shows its scope, last audited successful read, and current
actor-specific grants for that resource. Older reads without timestamps show
**unknown**. The grant list reports existing decisions; agent and app-tool
reads retain their existing permission gates.

Usage timestamps cover successful permission-audited reads for that scope.
Direct clipboard copies of global/workspace secrets and terminal-environment
injection do not add usage timestamps.

When the list is empty, choose the destination scope and **Import .env…**
(`i`). Pick a file containing literal `NAME=value` assignments. The importer
accepts comments, an `export` prefix, and single- or double-quoted values.
Shell expansion and multiline values are unsupported. Import replaces
matching names in the selected scope and leaves the source file unchanged.
The complete file is validated before writing; a storage failure reports
how many entries were saved.

From a terminal:

```sh
plexi secret set MY_API_KEY
```

Plexi prompts you to type the value (hidden). To read the value from an existing environment variable instead:

```sh
plexi secret set MY_API_KEY --from-env
```

A stored secret is not added to terminal environments. To give one to the
terminals you open in this workspace, turn on terminal injection for it in
Secrets (Cmd+Shift+S). Panes Plexi opens for an agent never receive workspace or
folder secrets in their environment.

To make a secret available across all projects (not just the current workspace):

```sh
plexi secret set MY_API_KEY --global
```

## Get a Secret

```sh
plexi secret get MY_API_KEY
```

Looks up the secret for the current project first, then falls back to the global store.

Inside a Plexi pane, any process could be an agent, so the host asks you first.
The Plexi window shows an approval, and the command prints `permission_required`
until you click **Allow once**. Run it again after the click. Allow once covers
one read.

## List Secrets

```sh
plexi secret list
```

Lists global and current-workspace secret names, plus folder entries when
`folder_secrets` is enabled. Values are not shown. Workspace names injected
into human terminals are marked. `--global` excludes workspace and folder
entries in both text and JSON output.

`plexi secret list --json` returns the app's names-only inventory, including
`scope`, `last_used` (Unix seconds or `null` when unknown), and `grant`
(actor-specific permission descriptions). Add `--global` to restrict it to
global secrets.

## Delete a Secret

```sh
plexi secret delete MY_API_KEY
```

To update a secret, run `plexi secret set` again with the new value. The old value is overwritten.

## Using Secrets in an App

Declare `secrets.get` in your app's `manifest.toml`:

```toml
[app.capabilities]
capabilities = ["secrets.get"]
```

SDK v3 apps use module-level `init(size, args)`, `update(event)`, and `view()`. The public Python SDK does not yet expose a module-level secret-read effect; until that lands, this page is the source of truth for CLI storage, workspace scoping, and manifest capability declaration.

## Workspace Scoping

Secrets are scoped by workspace root path — a secret set inside one project is not visible to another project at a different path. Use `--global` to share a secret across all workspaces on the machine.

### Human Pane Authority and Context Boundary

A human-owned terminal pane (a shell opened by the host for you, with no agent head or agent role attached and not spawned by an agent) carries your user authority by default, **scoped to that pane's context root**:

- **Context isolation (including nested directories):** Subdirectory contexts are isolated units. A pane in context A has no authority over context B's secrets, heads, or grants, including when context B is nested inside A's directory. Attempts to read, list, or grant secrets or heads belonging to another context root are refused (`permission_denied`).
- **Agents are narrowed by grants:** Agents and heads created from a human-owned pane receive only the grants explicitly passed with `--grant` (checked as a subset of the human pane's context authority). An agent-spawned pane (including a plain shell created by an agent via `plexi pane new`) never inherits human authority and requires explicit grants.
- **Not an OS file-access boundary:** This boundary governs the Plexi API (`agents_api` and host tools), secrets, and grant scoping. It is **not** an operating-system filesystem sandbox: a process running in the pane still runs as your OS user and can read or write any file on disk that your OS user can access.

## Linux storage

Global, workspace, and folder secrets use Secret Service through `secret-tool`
when available, or a labeled AES-256-GCM encrypted-file fallback. The selected
backend is recorded in `folder-secrets-backend.txt` in the channel profile and
reused; a later service outage does not silently select a different backend.

The fallback stores ciphertext in `folder-secrets.enc` in the channel profile.
Its key is `folder-secret.key` under the operating system's local data directory
in `plexi`, outside the profile. This is encrypted file storage, not an OS
keyring or isolation from other processes running as the same user.

`PLEXI_FOLDER_SECRETS_BACKEND=encrypted-file-fallback` forces the fallback for
the process without probing Secret Service or changing the recorded choice.
An unsupported override logs a warning and also uses the fallback without
contacting Secret Service. The override applies to global and workspace
secrets as well as folder secrets; `folder_secrets` controls the folder scope,
not the backend used by global or workspace secrets.

On the first global or workspace store operation in a process, Plexi attempts
to copy a legacy plaintext `secrets.json` into the selected backend. These are
upserts: a legacy account replaces the same account in the destination. Plexi
removes the legacy file only after every write succeeds. A read or write failure
is logged and leaves the source intact; partial destination writes may remain.
A removal failure is logged and also leaves the source. Migration is attempted
once per process, so retrying requires a new process. New secrets use the
selected backend, never the legacy plaintext file.
