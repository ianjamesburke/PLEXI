# Plexi Secrets Manager

**Status:** Proposal — partially v2.0 scoped (see below)  
**Last updated:** 2026-04-15

---

## Overview

Plexi manages secrets on behalf of agents and apps. Secrets never sit in plaintext on disk — values are stored in the macOS Keychain. Only Plexi's privileged broker process can read them; agents receive values at runtime via a narrow IPC channel and never touch the Keychain directly.

---

## Storage

**Values** — stored in the macOS Keychain under Plexi-namespaced keys. Never written to any file.

**Metadata** — a file at `~/.plexi/secrets/index.toml` tracks which keys exist and their scope (global or a directory path). This file contains no values — only key names and scopes. Low-sensitivity.

```toml
# ~/.plexi/secrets/index.toml
[[secret]]
key = "OPENAI_KEY"
scope = "global"

[[secret]]
key = "ANTHROPIC_KEY"
scope = "global"

[[secret]]
key = "STRIPE_SECRET"
scope = "/Users/ian/projects/billing"
```

---

## CLI

```sh
# Set a secret scoped to the current working directory
plexi secrets set OPENAI_KEY sk-...

# Set a global secret (available in all directories)
plexi secrets set --global OPENAI_KEY sk-...

# List secrets visible in the current directory (names only, no values)
plexi secrets list

# Remove a secret
plexi secrets unset OPENAI_KEY
plexi secrets unset --global OPENAI_KEY

# Show which scope a key resolves from in the current directory
plexi secrets which OPENAI_KEY
```

The mental model is `git config` vs `git config --global` — developers already know it.

---

## Inheritance

At agent/app launch, Plexi resolves secrets for the target directory using this chain:

1. Directory-scoped secrets for the exact launch path
2. Global secrets

Directory wins over global for the same key name. If a key exists at both levels, the directory value is used. If only at global, the global value is used.

This mirrors how Unix environment variables work — override at the nearest scope, inherit the rest.

---

## Runtime Injection

Secrets are injected via the Plexi broker IPC, not as files or persistent env vars:

1. Plexi launches an agent/app subprocess with a Seatbelt sandbox profile (scoped to its directory).
2. Before handing off, the broker resolves the secret chain for that directory.
3. The broker reads each required value from Keychain (privileged, user-authorized).
4. Values are written into the subprocess environment via a pipe, then the pipe is closed.
5. The subprocess receives the values in memory. They are gone when the process ends.

The agent process never has a file path to any secret. It never touches Keychain. The broker is the only Keychain reader.

---

## Manifest Declaration

Apps and agents declare which secrets they need in their `manifest.toml`. The broker only injects declared secrets — an agent cannot request arbitrary keys.

```toml
# manifest.toml
[secrets]
required = ["OPENAI_KEY"]
optional = ["ANTHROPIC_KEY"]
```

If a `required` secret is missing at launch, Plexi fails fast with a clear error naming the missing key. It does not silently proceed with a missing credential.

---

## Plexi IQ Pro Integration

For managed subscribers, Plexi injects its own API keys at global scope. The user never sees or manages these keys. Directory-level secrets can still override them — e.g., a user can pin a specific project to their own Anthropic key by setting `ANTHROPIC_KEY` at directory scope, which wins over the Plexi-managed global.

This makes the free tier (BYOK) and Pro tier (managed keys) use identical injection infrastructure. The only difference is who set the global keys.

---

## Security Properties

- **No plaintext on disk** — values live only in Keychain and in process memory during use.
- **Scope-limited injection** — agents only receive secrets declared in their manifest and visible in their directory scope.
- **Kernel-enforced boundary** — the agent sandbox (see `proposals/agent-sandbox.md`) prevents the agent process from reading Keychain or the secrets index directly.
- **Audit trail** — every broker injection is logged (key name, agent, timestamp — no values) to `~/.plexi-alpha/plexi.log`.

---

---

## Release Targeting

**v2.0 (scoped):** `plexi secrets` CLI + Keychain storage + global/directory scoped resolution + `index.toml` metadata. The runtime `SecretGet` app API already exists (`app_api.rs`); this adds the user-facing management surface and wires it to the same Keychain backend. Manifest declaration is forward-compatible — apps can declare `[secrets]` now, broker ignores it until injection lands.

**v2.1+ (deferred):** Pre-launch broker injection via pipe (§ Runtime Injection). Requires the agent sandbox (`proposals/agent-sandbox.md`) to be enforced or the security boundary is aspirational. The two models are compatible — injection just pre-populates what `SecretGet` would have fetched at runtime.

**Compatibility note:** `SecretGet` stays as the runtime pull API. It is not replaced — injection supplements it for apps that prefer declarative secrets over imperative requests.

---

## Out of Scope (this proposal)

- `--scope <path>` for setting secrets on a path you're not currently in — v2+
- Secret rotation / expiry — v2+
- Team-shared secrets (SpacetimeDB sync) — long-term, see `proposals/sync-architecture.md`
- Windows / Linux Keychain equivalents (libsecret, Windows Credential Store) — post-macOS
