---
title: Secrets
description: Store and access secrets inside Plexi apps.
verified_version: "3.6.19"
order: 6
---

Plexi has a built-in secrets store scoped to each workspace. Secrets are encrypted at rest via your system keychain and can be injected into apps that declare the `secrets.read` capability.

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

Declare `secrets.read` in your app's `manifest.toml`:

```toml
[app.capabilities]
capabilities = ["secrets.read"]
```

Then read secrets from `on_init` via the emitter:

```python
from plexi_sdk import App, RenderContext


class MyApp(App):
    async def on_init(self, ctx: RenderContext) -> None:
        self.api_key = await self.emit.get_secret("MY_API_KEY")

    def on_render(self, ctx: RenderContext) -> None:
        if self.api_key is None:
            ctx.text("Secret not set", color="#f59e0b")
            return
        # use self.api_key...


MyApp().run()
```

If the secret doesn't exist or the app lacks the capability, `get_secret()` returns `None`.

## Workspace Scoping

Secrets are scoped by workspace root path — a secret set inside one project is not visible to another project at a different path. Use `--global` to share a secret across all workspaces on the machine.
