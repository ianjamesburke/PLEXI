# Unified Permissions Broker Spec

Status: architecture spec.
Parent: [`agent-platform.md`](agent-platform.md).
Last updated: 2026-06-11.

Plexi currently has an app capability permission model: app id, workspace root, capability, and Green/Yellow/Red state. That model is a good base, but it is too narrow for agents, app connectors, event subscriptions, model routing, and undo.

This spec generalizes the current app permission store into one host broker. It does not create an Assistant-specific permission system.

## Current Landscape

Current permissions are centered on PGAP app capabilities:

- Actor: app id.
- Scope: workspace root.
- Target: `Capability` enum value such as `ai.query`, `panes.read`, `panes.control`, `fs.write`.
- State: `Green`, `Yellow`, `Red`.
- Persistence: permission store keyed like `app_id::workspace_path::capability`.
- UI: capability prompt modal with grant once, grant forever, deny once, deny forever.

That is enough for app startup and PGAP requests. It cannot express:

- agent-specific permissions
- app connector tools such as `chess.make_move`
- event subscriptions such as `chess.turn.ready`
- trigger modes such as context-only vs wake-agent
- model tier escalation or concrete model pointers
- rollback permission
- document/game-scoped grants
- package identity binding

## Target Model

The broker evaluates a request:

```text
actor -> action -> target -> scope -> decision
```

Actor types:

- `app`
- `agent`
- `system`
- `managed_policy`

Target types:

- `capability`
- `host_tool`
- `app_connector`
- `app_event_stream`
- `mcp_tool`
- `file_scope`
- `secret`
- `package`
- `model_pointer`
- `undo_checkpoint`

Decision values:

- `allow`
- `ask`
- `deny`

Durations:

- `once`
- `session`
- `workspace`
- `document`
- `game`
- `path`
- `package_identity`
- `always`

Risk classes:

- read host state
- read app state
- subscribe to app events
- trigger agent turns from app events
- write app state
- write files
- send terminal input
- spawn process
- network request
- secret access
- install package
- model cost escalation
- rollback state
- destructive change

## Grant Record

The persisted shape should support the old app model and the new agent model:

```text
actor_type: app | agent | system | managed_policy
actor_id: stable id
actor_scope: built-in | user | workspace | marketplace | managed
workspace_root: path or null
target_type: capability | host_tool | app_connector | app_event_stream | mcp_tool | file_scope | secret | package | model_pointer | undo_checkpoint
target_id: stable target id
resource_scope: workspace | pane | document | game | path | package_identity | account | global
resource_id: optional stable id/path
decision: allow | ask | deny
duration: once | session | workspace | document | game | path | package_identity | always
source: managed | user | workspace | session
created_at: timestamp
expires_at: optional timestamp
```

The old key `app_id::workspace_path::capability` can migrate into:

```text
actor_type=app
actor_id=<app_id>
workspace_root=<workspace_path>
target_type=capability
target_id=<capability>
resource_scope=workspace
decision=<green/yellow/red mapped to allow/ask/deny>
```

## Settings vs Grants

Agent or app settings can request defaults. They do not grant themselves power.

`settings.toml`:

```toml
[permissions]
default_posture = "review"

allow = ["host.panes.read"]
ask = ["app.chess.make_move"]
deny = ["host.secrets.read"]

[[subscriptions]]
app = "chess"
events = ["turn.ready", "move.played"]
payload = "full"
trigger = "conversation"
default = "ask"
```

Permission store:

```text
agent=chess-opponent
target=app_event_stream:chess.turn.ready
decision=allow
resource_scope=game
duration=session
```

Settings say what the agent wants. The broker records what the user allowed.

## Broker Flow

For every sensitive action:

1. Build a typed permission request.
2. Evaluate managed deny.
3. Evaluate user/workspace/session deny.
4. Evaluate actor-specific deny.
5. Evaluate managed ask.
6. Evaluate user/workspace/session ask.
7. Evaluate actor-specific ask.
8. Evaluate persisted grants.
9. Evaluate actor-specific allow.
10. Evaluate user/workspace allow.
11. Fall back to default posture.

Deny wins over ask. Ask wins over allow. Managed policy cannot be overridden by user settings.

## UI Requirements

`/permissions` is one surface with filters:

- actor: app, agent, system
- target: app connector, event stream, host tool, file, secret, model, undo
- source: managed, user, workspace, session
- state: allow, ask, deny
- recent decisions
- recent denials

The host Agent pane should show active subscriptions:

- green: allowed
- yellow: ask first or pending
- red: denied
- gray: available but inactive

Clicking a subscription opens the host permission sheet. Agents cannot flip their own grants.

## Done When

- Existing app capability decisions migrate into the generalized grant model.
- PGAP app capability checks call the unified broker.
- Agent tool calls call the unified broker.
- App event subscriptions call the unified broker.
- Mutating app connector calls call the unified broker.
- Model tier escalation can be gated by the broker.
- Undo/rollback operations can be gated by the broker.
- The permission management UI reads and writes one permission store.
