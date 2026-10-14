---
title: Secrets
description: Store and access secrets inside Plexi apps.
order: 6
---

Plexi has a built-in secrets store scoped to each workspace. On macOS, secrets
are stored through the system keychain. On Linux, they are stored in a
mode-`0600` file in the Plexi profile; that file is not encrypted at rest.
Secrets are available to apps through the host-brokered `secrets.get`
capability.

## Set a Secret

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

Lists all stored secret keys. Values are not shown. Names injected into
terminals are marked.

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
