---
title: Secrets
description: Store and access secrets inside Plexi apps.
verified_version: "3.6.19"
order: 6
---

Plexi has a built-in secrets store scoped to each build channel. Secrets are encrypted at rest and injected into apps that declare the `secrets` capability.

## Set a Secret

```sh
plexi secrets set MY_API_KEY sk-abc123
```

Secrets are stored in your channel's profile directory (`~/.plexi/`, `~/.plexi-alpha/`, etc.) and never leave the machine.

## Get a Secret

```sh
plexi secrets get MY_API_KEY
```

## List Secrets

```sh
plexi secrets list
```

Lists all stored secret keys. Values are not shown.

## Using Secrets in an App

Declare `secrets = true` in your app's `manifest.toml`:

```toml
[capabilities]
secrets = true
```

Then read secrets via the SDK inside your draw handler:

```python
@app.on_draw
def draw(ctx):
    key = ctx.secret("MY_API_KEY")
    if key is None:
        ctx.text("Secret not set", color="#f59e0b")
        return
    # use key...
```

The host injects secrets at draw time. If the secret doesn't exist, `ctx.secret()` returns `None`.

## Channel Isolation

Secrets are isolated per channel. A secret set via `plexi secrets set` on the stable channel is not visible to `plexi-alpha` or a PR build. This means you may need to set secrets in each channel you use.

## Rotation

To update a secret, run `plexi secrets set` again with the new value. The old value is overwritten.

To delete a secret:

```sh
plexi secrets delete MY_API_KEY
```
