# Personal sign-off

A tool call is one of three approval tiers:

- **Click.** The existing approval. This is the tier when nothing requires personal sign-off.
- **Each time.** A fresh signature on every execution. The signature does not become a reusable grant.
- **Time-boxed.** One signature mints a revocable grant. The default lifetime is 7 days (`time_boxed_ttl_secs = 604800`, clamped to 60 seconds through 30 days). Revoking the grant makes the next call ask again.

A tool manifest sets this with `requires = "personal_signoff"` and an optional `signoff = "each_time"` or `"time_boxed"`. An unknown or absent `signoff` stays on each-time. A grant row can carry the same marker (`requires_personal_signoff`). A click, a posture allow, and an unsigned grant cannot lower a requirement. A matching deny still wins.

## What the signature covers

The host builds one canonical message, `plexi-signoff-v1`, and asks the signer to sign those bytes. The gate verifies that signature before the tool runs. No fresh valid signature means the call is refused and audited.

The message binds the actor (type, scope, trust origin, id), resource, action, argument fingerprint, nonce, expiry, the short submit deadline, the package, and the tier. The gate rebuilds that message from the stored grant on every use, so an edited expiry, actor, or argument hash no longer verifies.

Each-time expiry is the submit deadline (two minutes). Time-boxed expiry is the grant lifetime; the submit deadline is still two minutes. A later call under a time-boxed grant may use different arguments. The signature is checked again, against the message stored with the grant.

## macOS

A signed macOS build creates one EC P-256 key per user in the Secure Enclave, labeled `plexi.personal-signoff`. The key's `SecAccessControl` is `biometryCurrentSet` with `privateKeyUsage`. If that access control cannot be created, the key is created with `userPresence` instead. `LAContext` sets the prompt reason. `SecKeyCreateSignature` is what asks the enclave to use the key, so a person may see the reason prompt and then the enclave prompt.

Cancelling Touch ID is a refusal. It does not fall through to a password. If the enclave key cannot be created (an unsigned dev build, or a machine without the enclave), the host follows `[permissions.personal_signoff] fallback`.

## Other builds

`fallback = "refuse"` is the default. The call is refused and audited, and the prompt is labeled as not Touch ID.

`fallback = "password"` asks for the OS password. On macOS that is the device-owner authentication dialog. On Linux it is a hidden `/dev/tty` prompt checked with `sudo -S -k -v`. The audit mechanism is `os_password`, and every label says this is not Touch ID. A host with no terminal refuses that prompt rather than pretending a password was entered. `sudo` configured with `NOPASSWD` does not prove a password was typed.

An unknown fallback, including `"click"`, refuses.

## Manual Touch ID check

This path needs a signed macOS build. The Linux and unsigned builds cannot exercise the Secure Enclave; their automated tests use a mock signer.

1. Sign the host and install that build.
2. Mark one tool, or a grant marker for it, with `requires = "personal_signoff"`. Leave chess tools unmarked.
3. Leave `[permissions.personal_signoff]` unset, or set `fallback = "refuse"`.
4. Invoke the tool. Expect the LAContext reason, then the enclave prompt (Touch ID). Approve it.
5. Confirm the audit row's mechanism is `touch_id` and the tool runs.
6. Cancel the prompt on a second call. The call is refused and audited. No password dialog appears.
7. For `signoff = "each_time"`, every later call prompts again. An old signature does not admit the new call.
8. For `signoff = "time_boxed"`, a later call with different arguments runs without a new prompt until the grant expires. Revoke that grant and confirm the next call prompts again.
9. Rebuild unsigned, or run where the enclave cannot create the key. The log says the enclave is unavailable. With `fallback = "password"`, the next prompt is the OS password dialog and is labeled as not Touch ID. With `fallback = "refuse"`, the call is refused and audited.
