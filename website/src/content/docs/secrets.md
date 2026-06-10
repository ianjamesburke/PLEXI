---
title: Secrets
description: Store and access secrets inside Plexi apps.
verified_version: "0.0.689"
order: 6
---

Plexi has a built-in secrets store scoped to each workspace. Secrets are stored through the system keychain and are available to apps through the host-brokered `secrets.get` capability.

## Set a Secret

```sh
plexi secret set MY_API_KEY
```

Plexi prompts you to type the value (hidden). To read the value from an existing environment variable instead:

```sh
plexi secret set MY_API_KEY --from-env
```

To make a secret available across all projects (not just the current workspace):

```sh
plexi secret set MY_API_KEY --global
```

## Get a Secret

```sh
plexi secret get MY_API_KEY
```

Looks up the secret for the current project first, then falls back to the global store.

## List Secrets

```sh
plexi secret list
```

Lists all stored secret keys. Values are not shown.

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

Then read secrets from an async hook via the emitter:

```python
from plexi_sdk import App
from plexi_sdk.ui import AppBar, Column, Label


class MyApp(App):
    def on_init(self) -> None:
        self.api_key = None

    async def on_key(self, key, mods):
        if key == "s":
            self.api_key = await self.emit.secret_get("MY_API_KEY")
            self.emit.schedule_render()

    def view(self):
        return Column([
            AppBar("Secrets"),
            Label("Secret loaded" if self.api_key else "Press s to request secret"),
        ])


MyApp().run()
```

If the secret does not exist or the app lacks the capability, `secret_get()` returns `None`.

## Workspace Scoping

Secrets are scoped by workspace root path — a secret set inside one project is not visible to another project at a different path. Use `--global` to share a secret across all workspaces on the machine.
