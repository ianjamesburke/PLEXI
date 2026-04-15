# Plexi Sync Architecture

Specification for directory, file, secrets, and collaborative editing synchronization across machines and users.

**Status:** Draft
**Date:** 2026-04-11

---

## 1. Overview

The sync architecture enables five capabilities:

1. **File synchronization** across machines for shared project directories
2. **Secrets sharing** with encrypted transport and explicit consent
3. **Collaborative editing** with multi-cursor support and conflict-free merging
4. **Inter-document communication** between apps running in different directories
5. **Offline-first operation** with guaranteed convergence on reconnect

The system is built on two foundational technologies:

- **Tailscale** — zero-config encrypted mesh VPN providing transport, identity, and access control
- **SpacetimeDB** — local-first database with CRDT state replication providing conflict-free data synchronization

No central server is required. All synchronization is peer-to-peer, coordinated through Tailscale's control plane and SpacetimeDB's replication protocol.

### Current State

Plexi's existing architecture provides the building blocks:

| Component | Current Implementation | Sync Implication |
|---|---|---|
| Secrets | macOS Keychain via `security` CLI, indexed at `~/.plexi-alpha/secrets-index.json` | Secrets are machine-local; sync must be opt-in |
| Workspace state | `.plexi/workspace.json` (directory-scoped) | Already directory-scoped; natural sync boundary |
| App state | `serialize_state()` / `restore_state()` on App trait, JSON transport | Transport-agnostic; ready for network replication |
| Permissions | Directory-scoped capabilities in `manifest.toml` | Sync inherits existing permission model |

---

## 2. Architecture Layers

```
+------------------------------------------------------------------+
|                    Layer 4: Inter-App Channels                    |
|          Named pub/sub channels, directory-scoped visibility      |
+------------------------------------------------------------------+
|                  Layer 3: Presence & Cursors                      |
|        Ephemeral subscriber state, debounced broadcasts           |
+------------------------------------------------------------------+
|              Layer 2: State Replication (SpacetimeDB)             |
|     Per-directory modules, CRDT text merge, LWW binaries          |
+------------------------------------------------------------------+
|                  Layer 1: Transport (Tailscale)                   |
|       WireGuard encryption, mesh VPN, identity & ACLs             |
+------------------------------------------------------------------+
```

### Layer 1: Transport — Tailscale

Tailscale provides the encrypted network layer between Plexi instances.

**Key properties:**
- Zero-config encrypted mesh VPN between machines
- Each Plexi instance registers as a Tailscale node
- Directory sharing uses direct node-to-node connections (no relay required for most topologies)
- No central server — peer-to-peer with Tailscale coordination plane
- Tailscale identity = user identity for all access control decisions

**Identity mapping:**
- Tailscale login identity (e.g., `ian@tailnet`) maps 1:1 to a Plexi sync identity
- ACLs defined in Tailscale control plane gate which nodes can communicate
- No separate auth system — Tailscale is the single source of truth for "who is this user"

### Layer 2: State Replication — SpacetimeDB

Each shared directory gets its own SpacetimeDB module. The module runs local-first and syncs when peers are connected.

**Core tables:**

```rust
#[spacetimedb::table(public)]
struct SyncedFile {
    #[primary_key]
    path: String,
    content: Vec<u8>,
    version: u64,
    last_modified: spacetimedb::Timestamp,
    modified_by: String,  // Tailscale identity
}
```

**Mutation rules:**
- All file mutations flow through SpacetimeDB reducers — never raw filesystem writes for synced state
- Text files use character-level CRDTs for conflict-free merging
- Binary files use last-write-wins with full version history for rollback
- Reducers: `insert_file`, `update_file`, `delete_file`, `rename_file`

### Layer 3: Presence & Cursors

Subscriber state tracks connected users in real time.

```rust
#[spacetimedb::table(public)]
struct Presence {
    #[primary_key]
    user_id: String,
    display_name: String,
    color: String,         // hex color, assigned on join
    active_file: String,
    cursor_line: u32,
    cursor_col: u32,
    selection_start_line: Option<u32>,
    selection_start_col: Option<u32>,
    selection_end_line: Option<u32>,
    selection_end_col: Option<u32>,
    last_seen: spacetimedb::Timestamp,
}
```

**Properties:**
- Ephemeral — not persisted to disk, only visible while connected
- Cursor position updates debounced to **50ms** to reduce bandwidth
- Presence rows auto-deleted when a user disconnects (SpacetimeDB `on_disconnect` reducer)

### Layer 4: Inter-App Channels

Named publish/subscribe channels scoped to the directory hierarchy.

```rust
#[spacetimedb::table(public)]
struct ChannelMessage {
    #[primary_key]
    #[auto_inc]
    id: u64,
    channel: String,
    sender_app_id: String,
    sender_dir: String,
    payload: String,       // JSON-encoded
    timestamp: spacetimedb::Timestamp,
}
```

**Visibility rules:**
- Channels bubble **up** the directory hierarchy: a child directory publishes, any parent `.plexi` directory can subscribe
- Apps **cannot** subscribe to sibling or child directory channels (isolation boundary)
- Root-level apps (`~/.plexi/`) can subscribe to **all** channels (global observability)

**Use case:** Narrator app in `/projects/client-a` publishes `render_complete` with `{"output_path": "output/final.mp4", "duration": 32.5}`. A dashboard app in `/projects/` (parent) receives the event and updates its project status view.

---

## 3. File Synchronization

### Sync Scope

Only files within `.plexi/` directories are synced by default. Users opt in to additional paths via `.plexi/sync.toml`:

```toml
[sync]
include = ["input/", "output/", "manifest.yaml"]
exclude = ["*.mp4", "*.wav", "node_modules/"]
max_file_size_mb = 50
```

**Large binary handling:**
- Files exceeding `max_file_size_mb` sync metadata only (path, size, hash, timestamp)
- Content available via on-demand pull: peer requests file, sender streams it over Tailscale
- Common exclusions (video, audio) are excluded by default — metadata syncs, content doesn't

### Conflict Resolution

| File Type | Strategy | Detail |
|---|---|---|
| Text files (`.md`, `.py`, `.rs`, etc.) | CRDT merge | Character-level, preserves both users' edits simultaneously |
| Config files (`.toml`, `.yaml`, `.json`) | Field-level LWW | Last-write-wins per field, with merge log for audit |
| Binary files | Last-write-wins | Full version history retained for rollback |
| Manifest files (`project.yaml`, etc.) | Field-level merge | Critical for shared manifests between editors |

### Offline Support

All mutations queue in a local append-only log while disconnected:

```
.plexi/sync-log.jsonl
```

**Reconnection flow:**
1. On reconnect, queued mutations replay through SpacetimeDB reducers
2. CRDT properties guarantee convergence regardless of replay order
3. Any prior state is reconstructable from the append-only log

**Log entry format:**
```json
{"ts": "2026-04-11T14:32:01Z", "op": "update", "path": "notes.md", "version": 42, "user": "ian@tailnet"}
```

---

## 4. Secrets Synchronization

**Design principle: secrets never leave the machine by default.**

### Model A — Declarative (v1)

Apps declare required secrets in their manifest. Each machine fills them independently via Plexi Secrets Manager. No secret values ever transit the network.

**`.plexi/secrets-manifest.toml`** lists required key names without values:

```toml
[[secrets]]
key = "ANTHROPIC_API_KEY"
description = "Anthropic API key for LLM calls"
required = true

[[secrets]]
key = "ELEVENLABS_API_KEY"
description = "ElevenLabs API key for TTS"
required = false
```

**Onboarding flow:**
When a new machine joins a shared directory, Plexi prompts:

> This project requires these secrets: `ANTHROPIC_API_KEY` (required), `ELEVENLABS_API_KEY` (optional). Set them up now?

The user enters values locally. Values go into macOS Keychain on that machine. Only the manifest (key names + descriptions) syncs — never values.

### Model B — Encrypted Sync (Future)

For teams that need actual secret value synchronization:

1. `.plexi/secrets.enc` — AES-256-GCM encrypted blob containing key-value pairs
2. Encryption key derived from a shared secret exchanged via Tailscale's WireGuard key exchange
3. On receiving machine: decrypt, store in local Keychain, zero plaintext from memory
4. Rotation: re-encrypt with new key, push to peers, peers re-import
5. Audit: every decrypt/import event logged to `.plexi/sync-log.jsonl`

### Scoping Rules

Secrets follow the `.plexi` directory hierarchy:

```
~/.plexi/                    # Root — secrets available everywhere
  secrets-index.json         # OPENAI_API_KEY, etc.

~/projects/client-a/.plexi/  # Project — overrides root
  secrets-manifest.toml      # ANTHROPIC_API_KEY (project-specific)

~/projects/client-a/narrator/.plexi/  # App — overrides project
  secrets-manifest.toml      # ELEVENLABS_API_KEY (app-specific)
```

Resolution order: most specific directory wins. A `OPENAI_API_KEY` set at the project level overrides the one at root.

The sync system syncs `.plexi/secrets-manifest.toml` only. Values never sync unless Model B is explicitly enabled for that directory.

---

## 5. Collaborative Editing

### SpacetimeDB Schema

```rust
#[spacetimedb::table(public)]
struct Document {
    #[primary_key]
    path: String,
    crdt_state: Vec<u8>,  // Serialized CRDT document
    version: u64,
}

#[spacetimedb::table(public)]
struct Cursor {
    #[primary_key]
    #[auto_inc]
    id: u64,
    user_id: String,
    file_path: String,
    line: u32,
    col: u32,
    selection_start_line: Option<u32>,
    selection_start_col: Option<u32>,
    selection_end_line: Option<u32>,
    selection_end_col: Option<u32>,
    color: String,
    last_updated: spacetimedb::Timestamp,
}
```

### Mutation Flow

```
User types          Local CRDT          SpacetimeDB         Remote users
    |                   |                    |                    |
    |---edit----------->|                    |                    |
    |   (instant        |                    |                    |
    |    local feedback) |                    |                    |
    |                   |---batch (100ms)--->|                    |
    |                   |                    |---broadcast------->|
    |                   |                    |                    |--apply
    |                   |                    |                    |  to local
    |                   |                    |                    |  view
```

1. User types in Plexi text view
2. Edit applied to local CRDT state immediately (zero-latency local feedback)
3. Local buffer accumulates edits, debounced at **100ms**
4. Debounced batch sent to SpacetimeDB reducer
5. Reducer applies CRDT merge, broadcasts to all subscribers
6. Remote users receive merged state, apply to their local CRDT + view
7. Cursor positions broadcast on a separate **50ms** debounce

### Remote Cursor Rendering

| Property | Behavior |
|---|---|
| Color | Assigned on join, stored in Cursor table, consistent across sessions |
| Cursor | Colored vertical bar at `(line, col)` |
| Selection | Colored highlight over the selection range |
| Label | Username tooltip on hover |
| Idle fade | Cursor fades at 30s inactivity, disappears at 5min |

### CRDT Library Selection

| Library | Use Case | Rationale |
|---|---|---|
| `yrs` (Yjs Rust port) | Text files | Battle-tested, used by many production collaborative editors, excellent performance |
| `automerge-rs` | Structured config/manifest files | Better primitives for maps and lists, natural fit for TOML/YAML/JSON structures |

**v1 recommendation:** `yrs` for all text editing. Evaluate `automerge` for structured data in Phase 2.

---

## 6. Inter-Document Communication

### Channel Protocol

**Publish (app to Plexi):**
```json
{
  "type": "channel_publish",
  "channel": "render_complete",
  "payload": {
    "output_path": "output/final.mp4",
    "duration": 32.5
  }
}
```

**Subscribe (app to Plexi, on init):**
```json
{
  "type": "channel_subscribe",
  "channels": ["render_complete", "cost_alert"]
}
```

**Receive (Plexi to app, when subscribed channel fires):**
```json
{
  "type": "channel_message",
  "channel": "render_complete",
  "sender_app_id": "narrator",
  "sender_dir": "/Users/ian/projects/client-a",
  "payload": {
    "output_path": "output/final.mp4",
    "duration": 32.5
  },
  "timestamp": "2026-04-11T14:32:01Z"
}
```

### Scoping Rules

```
~/.plexi/                              # Root — can subscribe to ALL channels
  |
  ~/projects/.plexi/                   # Can subscribe to own + children's channels
  |   |
  |   ~/projects/client-a/.plexi/      # Can subscribe to own channels only
  |   |   |
  |   |   ~/projects/client-a/narrator/.plexi/  # Publishes up, cannot see siblings
  |   |
  |   ~/projects/client-b/.plexi/      # Cannot see client-a's channels
```

- Apps publish to any channel name
- Apps subscribe to channels in their own directory or any **ancestor** `.plexi` directory
- Apps **cannot** subscribe to sibling or descendant directory channels (isolation)
- Root-level apps (`~/.plexi/`) subscribe to all channels (global observability use case)

---

## 7. Plexi Sync App

A privileged built-in app for managing all sync configuration.

**Required capabilities:** `network`, `system`

The `system` capability is a new elevated capability granting access to Tailscale configuration and SpacetimeDB module management. Only the Sync app and explicitly trusted apps receive it.

### UI Screens

| Screen | Content |
|---|---|
| **Peers** | Connected Tailscale nodes, online/offline status, last sync timestamp |
| **Shared Directories** | Which directories are shared, with whom, sync status (synced / pending / conflict) |
| **Conflicts** | Queue of unresolved binary file conflicts (text conflicts resolve automatically via CRDT) |
| **Secrets** | Declared secrets, local fill status, prompt to fill missing required secrets |
| **Activity** | Recent sync events: file changes, secret access, user join/leave |

### Setup Flow

```
1. User opens Plexi Sync app
2. App checks: `tailscale status`
   |
   +-- Not installed --> Show install instructions, link to tailscale.com
   |
   +-- Installed, not logged in --> Show `tailscale login` prompt
   |
   +-- Installed, logged in:
       |
       3. Display connected nodes and Tailscale identity
       4. User selects directory to share
       5. App creates `.plexi/sync.toml` with defaults
       6. User invites peer (by Tailscale identity)
       7. Peer receives invitation in their Plexi Sync app
       8. Peer accepts --> SpacetimeDB module initialized for the shared directory
       9. File sync begins according to `.plexi/sync.toml` rules
```

---

## 8. Security Model

### Threat Model

| Layer | Protection |
|---|---|
| Transport | Tailscale / WireGuard — all traffic encrypted end-to-end |
| Authentication | Tailscale identity — no separate auth system |
| Authorization | Per-directory ACLs in `.plexi/sync.toml` + Tailscale ACLs |
| File access | Directory-scoped permissions — apps cannot escape their scope |
| Secrets | Model A: never transit. Model B: AES-256-GCM encrypted, key via WireGuard exchange |

### Permission Escalation

- Sharing a directory requires **explicit user action** in the Sync app — never automatic
- Each shared directory shows exactly who has access (Tailscale identities)
- Revoking access: remove peer from SpacetimeDB subscribers + Tailscale ACL update
- All sync events logged to `.plexi/sync-log.jsonl` as an audit trail

### Default Exclusions

Sensitive file patterns are excluded from sync **even if the parent directory is shared**:

```
.env*
credentials*
*.key
*.pem
*.p12
*.secret
id_rsa*
id_ed25519*
```

These defaults apply unless explicitly overridden in `.plexi/sync.toml`. Apps with `env_file_access = false` in their manifest cannot read synced sensitive files even if the override is present.

---

## 9. Configuration Files

### `.plexi/sync.toml` (per-directory)

```toml
[sync]
enabled = true
module_id = "spacetime_abc123"  # SpacetimeDB module ID for this directory

[sync.files]
include = ["**/*"]
exclude = ["*.mp4", "*.wav", "*.mov", "node_modules/", ".env*"]
max_file_size_mb = 50

[sync.peers]
allowed = ["ian@tailnet", "francisco@tailnet"]

[sync.secrets]
mode = "declarative"  # "declarative" | "encrypted"
```

All fields are required when `sync.enabled = true`. No defaults — missing fields fail with a clear error at sync startup.

### `~/.plexi-alpha/config.toml` additions

```toml
[sync]
enabled = true
tailscale_path = "/usr/local/bin/tailscale"
spacetimedb_url = "https://spacetimedb.example.com"  # or local instance
default_crdt_library = "yrs"

[sync.global_excludes]
patterns = [".env*", "credentials*", "*.key", "*.pem", "*.p12"]
```

### `.plexi/secrets-manifest.toml`

```toml
[[secrets]]
key = "ANTHROPIC_API_KEY"
description = "Anthropic API key for LLM calls"
required = true

[[secrets]]
key = "ELEVENLABS_API_KEY"
description = "ElevenLabs API key for TTS"
required = false
```

---

## 10. Implementation Phases

### Phase 1: File Sync (MVP)

**Scope:**
- Tailscale transport integration
- File watching via `fsnotify` + SpacetimeDB tables for file metadata
- Last-write-wins for **all** file types (no CRDT yet)
- `.plexi/sync.toml` configuration
- Sync app with Peers and Shared Directories screens
- Offline queue in `.plexi/sync-log.jsonl`

**Not in scope:** CRDT merging, cursors, presence, secrets sync, inter-app channels.

**Ships when:** Two machines can share a `.plexi/` directory and see file changes propagate within 5 seconds.

### Phase 2: Text CRDT

**Scope:**
- `yrs` integration for text files
- Character-level merge for: `.md`, `.yaml`, `.toml`, `.json`, `.py`, `.rs`, `.ts`, `.js`
- Conflict-free simultaneous editing without merge UI
- Sync app Conflicts screen (for binary conflicts only — text auto-resolves)

**Ships when:** Two users editing the same `.md` file see each other's changes merge without conflicts.

### Phase 3: Presence & Cursors

**Scope:**
- Cursor position broadcasting via SpacetimeDB ephemeral state
- Remote cursor rendering in Plexi text views (colored bars, selection highlights)
- User color assignment and presence indicators
- Idle fade and disconnect cleanup

**Ships when:** User A sees User B's cursor moving in real time in a shared file.

### Phase 4: Secrets Sync (Model B)

**Scope:**
- Encrypted secret blob sync (`.plexi/secrets.enc`)
- Key exchange via Tailscale secure channel
- Decrypt-to-Keychain import flow
- Key rotation protocol
- Audit logging for all secret access events

**Ships when:** A secret set on Machine A appears in Machine B's Keychain after explicit consent on both sides.

### Phase 5: Inter-App Channels

**Scope:**
- Pub/sub channel system via SpacetimeDB tables
- Directory-scoped visibility rules (bubble up, no sibling/child access)
- Channel protocol (publish, subscribe, receive messages)
- Root-level global observability subscription

**Ships when:** An app in a child directory publishes an event and a dashboard app in a parent directory receives it.
