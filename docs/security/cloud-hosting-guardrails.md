# Cloud hosting guardrails

Must-haves before Plexi runs containerized agents (a per-tenant Fly Machine, or anything that replaces the desktop as the place an agent executes). The phone relay is a router. It is not this boundary. Design these in before that build starts.

The relay threat model is `relay-threat-model.md`. The assistant authority gap (grants that do not bind to arguments) is `docs/assistant-authority-model.md`. This list does not restate either one.

## Tenant isolation

One hard boundary per tenant: a separate machine, or a container runtime that does not share a kernel user namespace, filesystem, or docker socket with another tenant. `host_id` is a routing key, not an isolation boundary. Tenant networks deny tenant-to-tenant traffic. A bug in one agent must not be a read or write of another tenant's disk, process, or relay session.

## Secrets stay in the keychain or the tenant vault

Host tokens, model keys, and workspace credentials never enter the relay, an image, a build arg, or a log. The desktop keeps its host token in the keychain. A cloud tenant gets a vault of its own. Injection into a machine is short-lived and scoped to that boot. The relay SQLite schema does not grow a secrets column. Compromising the relay yields token hashes and in-flight bodies, not a reusable model key.

## Egress policy

Default deny from a tenant machine. Allow the relay and the model endpoint that tenant is configured to use. No arbitrary outbound, no link-local metadata address, no path that lets one tenant open a socket to another tenant's machine.

## The permission gate stays per tenant

The gate runs inside the tenant boundary, on the host that holds authority. The relay, the runner, and a phone cookie cannot approve, widen a grant, or skip `waiting_for_permission`. Cloud turns enter the same `submit_assistant_turn` path the desktop uses. A runner-level "auto approve" flag does not exist.

## Audit

Append-only, tenant-scoped record of pair, session issue, revoke, turn accept (ids and sizes, never bodies), permission ask, and grant decision. Operator access to the audit stream is itself an audit event. The relay's info log is not this record.

## 30-day retention

Paired devices already drop after 30 days idle (`DEVICE_IDLE_SECONDS` in the relay). `PairingRegistry.retain` applies that ceiling to host rows that have a `created` timestamp and to queued-envelope metadata (`request_id`, `host_id`, `queued_at`, `size` — no body). Host rows written before `created` existed stay. The desktop job in `cloud::retention` prunes logs Plexi controls on the same startup path as log rotation, and prunes local ledger rows and assistant conversations only when `[cloud] retain_local_history` is set. Cloud audit and any stored prompt follow that same ceiling unless the user exports them. Undelivered relay bodies stay on the 120 second TTL and are not copied into the tenant store. Paired devices already drop after 30 days idle (`DEVICE_IDLE_SECONDS` in the relay). Cloud audit and any stored prompt follow that same ceiling unless the user exports them. Undelivered relay bodies stay on the 120 second TTL and are not copied into the tenant store.

## No shared credentials

No global model key, no shared SSH key, no host token reused across tenants. One credential per tenant, rotatable, and revoked when the tenant is. Compromise of one tenant reveals nothing that authenticates another.
