# Cloud hosting guardrails

Must-haves before Plexi runs containerized agents (a per-tenant Fly Machine, or anything that replaces the desktop as the place an agent executes). Design these in before that build starts.

The assistant authority gap (grants that do not bind to arguments) is `docs/assistant-authority-model.md`. This list does not restate either one.

## Tenant isolation

One hard boundary per tenant: a separate machine, or a container runtime that does not share a kernel user namespace, filesystem, or docker socket with another tenant. `host_id` is a routing key, not an isolation boundary. Tenant networks deny tenant-to-tenant traffic. A bug in one agent must not be a read or write of another tenant's disk, process, or session.

## Secrets stay in the keychain or the tenant vault

Host tokens, model keys, and workspace credentials never enter an image, a build arg, or a log. The desktop keeps its host token in the keychain. A cloud tenant gets a vault of its own. Injection into a machine is short-lived and scoped to that boot.

## Egress policy

Default deny from a tenant machine. Allow the model endpoint that tenant is configured to use. No arbitrary outbound, no link-local metadata address, no path that lets one tenant open a socket to another tenant's machine.

## The permission gate stays per tenant

The gate runs inside the tenant boundary, on the host that holds authority. The runner cannot approve, widen a grant, or skip `waiting_for_permission`. Cloud turns enter the same `submit_assistant_turn` path the desktop uses. A runner-level "auto approve" flag does not exist.

## Audit

Append-only, tenant-scoped record of pair, session issue, revoke, turn accept (ids and sizes, never bodies), permission ask, and grant decision. Operator access to the audit stream is itself an audit event.

## 30-day retention

The desktop job in `cloud::retention` prunes logs Plexi controls on the startup path and prunes local ledger rows and assistant conversations only when `[cloud] retain_local_history` is set. Cloud audit and stored prompts follow the same 30-day ceiling unless exported.

## No shared credentials

No global model key, no shared SSH key, no host token reused across tenants. One credential per tenant, rotatable, and revoked when the tenant is. Compromise of one tenant reveals nothing that authenticates another.
