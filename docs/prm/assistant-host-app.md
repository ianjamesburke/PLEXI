# Host Assistant App Spec

Status: product and architecture spec.
Parent: [`app-framework-marketplace.md`](app-framework-marketplace.md).
Last updated: 2026-06-11.

This document defines the first-party Plexi Assistant as a host app, not a PGAP app. It is the workspace operator: it can reason about panes, apps, files, permissions, installed skills, and app-exposed tools because the host owns those things.

The current `apps/assistant/` PGAP should become a reference app or be retired after the host Assistant covers the same chat behavior. Third-party agent apps can still exist, but they do not get ambient workspace control.

## Research Inputs

The design mirrors Claude Code where the model fits Plexi:

- Claude Code treats slash commands as session controls and prompt workflows. Commands are only recognized at the start of a message, support filtering from `/`, and pass trailing text as arguments. Source: <https://code.claude.com/docs/en/commands>
- Claude Code skills are markdown-backed procedures that load only when used or when auto-selected. Custom commands now map to skills, while legacy command files still work. Source: <https://code.claude.com/docs/en/skills>
- Claude Code settings have managed, user, project, and local scopes. Normal settings override by scope; permission rules merge and are evaluated by the runtime, not by the model. Source: <https://code.claude.com/docs/en/settings>
- Claude Code permissions use allow, ask, and deny rules. Deny wins before ask, ask wins before allow, and `/permissions` is the user-facing editor. Source: <https://code.claude.com/docs/en/permissions>
- Claude Code permission modes set the baseline: read-only/default, plan, accept-edits, auto, dont-ask, and bypass. Plexi should not clone every mode by name, but it needs the same distinction between baseline posture and per-tool grants. Source: <https://code.claude.com/docs/en/permission-modes>
- Claude Code hooks run at lifecycle points such as session start, prompt submit, command expansion, pre-tool-use, permission request, post-tool-use, compaction, and session end. Source: <https://code.claude.com/docs/en/hooks>
- Claude Code plugins package skills, commands, agents, hooks, MCP servers, LSP servers, monitors, and themes. Plexi needs the same package shape, but with app connectors and host tools as first-class components. Source: <https://code.claude.com/docs/en/plugins-reference>
- Claude Code uses MCP for external tools and data sources. Plexi should support MCP, but Plexi apps should expose typed host-mediated capabilities directly instead of pretending every local app is an MCP server. Source: <https://code.claude.com/docs/en/mcp>
- Claude Code can start from its own system prompt preset, append product-specific instructions, or replace the prompt entirely. Plexi needs named agents with system prompts, model settings, tools, and permission posture. Source: <https://code.claude.com/docs/en/agent-sdk/modifying-system-prompts>
- OpenClaude adds a useful prior for Plexi's agent registry: agents are markdown files with frontmatter for name, description, tools, model, effort, permission mode, MCP servers, hooks, max turns, skills, background behavior, isolation, and optional memory. It also supports per-agent model routing through `agentModels` and `agentRouting`. Source: <https://github.com/Gitlawb/openclaude>

## Product Goal

The host Assistant is the place where a user asks Plexi to do work across the workspace:

- answer questions using `ai.query`
- read panes and app state
- open apps and terminals
- drive app UIs through typed actions
- call app-exposed tools
- install and use skills, system prompts, and tool collections
- ask for one-time or persistent grants before making changes
- leave a local audit trail of every permission decision and meaningful action

The Assistant should feel like Claude Code inside Plexi, but its power model is different. Claude Code drives a repository through shell and file tools. Plexi Assistant drives a local computing environment through panes, PGAP apps, host APIs, and app-published tools.

## Non-Goals

- Do not make third-party PGAP apps privileged by default.
- Do not describe Python apps as sandboxed.
- Do not route the host Assistant through `PLEXI_SOCKET` or the CLI to control panes.
- Do not require a hosted account for local Assistant use, local skills, local app tools, or user-owned AI keys.
- Do not build paid skill enforcement before local package install and hosted marketplace metadata are stable.

## Placement

The Assistant is a first-party host app. It appears as a pane and uses host UI primitives, but it runs in-process with explicit host boundaries instead of as a child PGAP process.

It should be implemented as a host module with pure state and effects:

- `AssistantModel`: conversations, input, selected command, active agent, selected tools, transient thinking/streaming state.
- `AssistantEffect`: AI query, tool call, app action, pane action, permission prompt, package install, session write.
- `AssistantStore`: local transcripts, compaction summaries, installed skill index, agent index, tool collection index.
- `AssistantRenderer`: host UI kit rendering for chat, command picker, tool call rows, permission sheets, settings, and audit links.

The model must be testable without egui. Rendering gets smoke coverage through `PlexiUiHarness`.

## Existing Infrastructure To Reuse

The host Assistant should build on the infrastructure from the `feature/assistant-chat-refactor` work instead of recreating it.

Reuse these primitives:

- Live AI streaming from `AiBroker::dispatch(..., on_delta)` so Assistant text appears while the turn is running, not after the final response.
- Reasoning/thinking deltas parsed from OpenRouter and carried separately from answer text.
- `AiStreamChunk` events with optional reasoning payloads as the model for host-to-UI streaming state, even if the host Assistant eventually consumes the broker directly rather than through PGAP.
- The SDK/UI learnings from the refactored chat composer: multiline input, Enter submit, Shift+Enter newline, live thinking display, bottom-aligned scroll, pinned-to-bottom behavior only during streaming, and persisted collapsed thinking markers.
- Text input live-edit and control-key events (`TextChanged`, `TextInputKey`) for slash command filtering, completion menus, history navigation, and double-`Escape` history.
- Headless scene coverage for idle and streaming Assistant states as the pattern for host Assistant UI regression tests.

Do not keep the privileged workspace operator as a PGAP app. The refactor's useful output is the streaming, composer, thinking, and scene-test infrastructure. The pane/app/terminal control path still moves into host-native tools and the unified permission broker.

## Core Concepts

**Agent**: A named Assistant persona with a system prompt, when-to-use description, model tier policy, optional exact model pointers, enabled tools, enabled skills, app connectors, permission posture, hooks, max-turn limits, and background/isolation behavior. Examples: Default, Builder, Spreadsheet Analyst, Writing Partner.

**Skill**: A prompt-backed workflow with optional supporting files and scripts. A skill can be invoked manually with `/skill-name` or auto-selected when its description matches the task.

**Tool collection**: A package of host tools, MCP tools, app connectors, and permission defaults. A spreadsheet automation pack might include `csv.read`, `csv.write_cell`, `csv.add_sheet`, and `csv.export`.

**App connector**: The host-side description of tools exposed by a running or installed app. It is derived from the app manifest plus runtime registration. The connector names schemas, scopes, mutability, and required grants.

**Grant**: A user decision that allows, asks, or denies a tool or app connector in a scope. Grants can last for one call, this session, this workspace, this app document, or always for the same signed package identity.

**Hook**: A user, project, marketplace, or managed callback that observes or controls Assistant lifecycle events.

**Model tier**: A user-facing intelligence level: `low`, `medium`, or `high`. Tiers are stable product concepts; the concrete model behind a tier can differ by agent and can change as providers improve.

**Model pointer**: An advanced setting that points an agent or tier to a concrete provider/model/backend configuration, such as OpenRouter model id, Ollama model id, local provider, or Plexi AI subscription route.

## Settings

Plexi Assistant needs its own settings tree. It should not reuse app settings or raw `config.toml` keys without a schema.

Settings scopes:

- Managed: future organization or device policy. Highest priority.
- User: `~/.plexi-<channel>/agents/settings.toml`, applies across contexts in that channel.
- Workspace: `<workspace_channel_dir>/agents/settings.toml`.
- Local workspace: `<workspace_channel_dir>/agents/settings.local.toml`, ignored by default.
- Session: temporary overrides from slash commands or UI controls.

The exact path helper must be channel-aware. Do not hardcode `.plexi/`.

Settings groups:

- `model`: default backend, model tier, fallback behavior, thinking display, cost budget.
- `agents`: selected default agent, palette visibility, and agent source ordering.
- `permissions`: default permission posture, grant rules, denied tools, protected paths, protected apps.
- `skills`: enabled skill sources, disabled bundled skills, skill listing budget.
- `tool_collections`: enabled packs, source marketplace, local enablement state.
- `hooks`: lifecycle hooks and allowed hook transports.
- `ui`: chat density, auto-scroll, composer mode, command palette behavior.
- `privacy`: transcript retention, redact patterns, whether prompts may include pane screenshots or app state.

Settings reload while the host is running where practical. A settings change emits an event and records the source file.

## Agent Registry

Agents are the primary user-facing unit. The host Assistant is the app surface; agents are the selectable workers inside it.

Agent locations:

- Built-in agents: compiled into Plexi and always available unless managed policy disables them.
- User agents: `~/.plexi-<channel>/agents/<agent-id>/`.
- Workspace agents: `<workspace_channel_dir>/agents/<agent-id>/`.
- Marketplace agents: installed packages copied into the channel or workspace agent store.
- Managed agents: future organization policy.

Agent file layout:

```text
agents/
  builder/
    AGENT.md
    settings.toml
  spreadsheet/
    AGENT.md
    settings.toml
  writing/
    AGENT.md
    settings.toml
```

`AGENT.md` holds the durable agent prompt and frontmatter. `settings.toml` holds mutable local configuration: model routing, enabled tools, permission posture, hooks, and UI metadata. Keep settings in TOML because Plexi's workspace config is TOML and because agents need a typed config file agents can edit safely.

Agent metadata:

- id and display name
- description / when-to-use
- color/icon
- system prompt
- model tier defaults
- exact model pointers for advanced users
- allowed tools and denied tools
- enabled skills
- enabled app connectors
- MCP servers
- permission posture
- permission grants scoped to this agent
- hooks
- max turns
- background behavior
- isolation mode

Agents can be global or workspace-scoped in the same way apps can. The Assistant palette shows both, grouped by scope:

- Built-in
- User
- Workspace
- Marketplace
- Managed

If two agents share an id, higher-priority scopes shadow lower-priority scopes. The palette must show that shadowing instead of silently hiding it.

### Agents, Skills, And App Connectors

Do not turn every workflow into an agent.

- Use an agent when the work needs a distinct role, prompt, model policy, tool set, permission posture, or background/isolation behavior.
- Use a skill when the work is a reusable procedure the current agent can run.
- Use an app connector when the Assistant needs to read or mutate state owned by a Plexi app.
- Use a tool collection when several tools/connectors should install and enable together.

A spreadsheet agent makes sense if it is a specialized worker with spreadsheet-specific reasoning, model tier policy, and a constrained connector/tool set. CSV import cleanup, pivot-table creation, formula auditing, or "normalize this sheet" should usually be skills. `csv.read_range` and `csv.write_range` are app connector tools.

## Model Routing

Most users should never choose exact model ids. They choose `low`, `medium`, or `high` intelligence, and the active agent maps that tier to a concrete model pointer.

Model tier semantics:

- `low`: cheap, fast, good for simple edits, summaries, routing, and app state reads.
- `medium`: default work tier for normal multi-step tasks.
- `high`: expensive, slower, used for planning, ambiguous work, code generation, data repair, or destructive changes.

Resolution order:

1. Managed policy.
2. Session override from `/model` or `/effort`.
3. Active agent `settings.toml`.
4. Workspace `agents/settings.toml`.
5. User `agents/settings.toml`.
6. Built-in Plexi defaults.

Advanced users can define model pointers:

```toml
[models.local-qwen]
provider = "ollama"
model = "qwen2.5-coder:7b"

[models.openrouter-sonnet]
provider = "openrouter"
model = "anthropic/claude-sonnet-4.5"

[models.plexi-high]
provider = "plexi-ai"
tier = "high"
```

Agents map tiers to pointers:

```toml
[agent.default.tiers]
low = "local-qwen"
medium = "openrouter-sonnet"
high = "plexi-high"

[agent.spreadsheet.tiers]
low = "local-qwen"
medium = "openrouter-sonnet"
high = "openrouter-sonnet"
```

The UI should expose the simple control first: `low`, `medium`, `high`. Advanced model pointers live behind an advanced settings panel.

Agents can switch tiers during a task when policy allows it. A switch from `low` to `high` should be visible in the transcript and audit log, with a short reason. Switching concrete providers or using a more expensive tier may require confirmation if cost settings require it.

Exact model pointers are for power users and package authors. Marketplace packages may recommend tier mappings but cannot silently write API keys or force a paid provider.

## Unified Permissions

Agents have individual permission scoping, but Plexi has one permission system.

An agent is a first-class actor in the host permission broker. The Assistant does not own a parallel grant engine. Every agent tool call, app connector call, file write, secret access, terminal input, package install, and model-cost escalation is checked by the same permission broker used for PGAP capabilities and host APIs.

Grant record shape:

```text
actor_type: app | agent | system
actor_id: app id or agent id
actor_scope: built-in | user | workspace | marketplace | managed
target_type: host_tool | app_connector | mcp_tool | file_scope | secret | package | model_pointer
target_id: stable capability/tool id
scope: call | session | workspace | document | path | package_identity | always
decision: allow | ask | deny
source: managed | user | workspace | session
```

Agent permission config lives in the agent's `settings.toml`, but persisted grants and audit events go through the unified Plexi permission store. The settings file expresses defaults and requested posture; the permission store records user decisions.

Example:

```toml
[permissions]
default_posture = "review"

allow = [
  "host.panes.read",
  "app.csv.describe_table",
]

ask = [
  "app.csv.write_range",
  "host.terminal.send_input",
]

deny = [
  "host.secrets.read",
]
```

Evaluation order:

1. Managed deny.
2. User/workspace/session deny.
3. Agent-specific deny.
4. Managed ask.
5. User/workspace/session ask.
6. Agent-specific ask.
7. Existing persisted grant.
8. Agent-specific allow.
9. User/workspace allow.
10. Default posture.

The UI must expose this as one permissions surface. `/permissions` can filter by agent, app, connector, tool, workspace, or recent denial, but it edits the same underlying permission model.

Agent-specific permissions are how a spreadsheet agent can be trusted to edit a focused CSV document while a general writing agent cannot. The grant should bind as narrowly as possible: agent id, app package identity, connector id, document/path, workspace, and duration.

## Memory Scope

Durable memory is not a v1 implementation target. The Assistant should be future-compatible with memory, but the first implementation should not depend on automatic memory discovery.

For now:

- `AGENT.md` is the durable prompt for one agent.
- `settings.toml` is the mutable config for that agent.
- Conversation persistence, compaction, and history are required.
- Learned long-term memory, directory walking, and automatic memory snapshots are deferred.

When memory lands, it must remain channel-aware and must not create a second task-state source of truth.

## Slash Commands

Slash commands are recognized only when `/` is the first non-whitespace character in the composer. Typing `/` opens a searchable command picker. Arguments after the command name are passed as a single raw string plus parsed tokens.

Built-in commands:

| Command | Purpose |
|---|---|
| `/help` | Show built-in commands, installed skills, and tool packs. |
| `/clear` | Start a new conversation in the same workspace. |
| `/resume` | Open a previous Assistant session. |
| `/compact` | Summarize older turns and keep the active task context. |
| `/context` | Show token use, loaded instructions, active pane/app context, and enabled tools. |
| `/memory` | Show agent prompt files and future memory state. |
| `/model` | Switch model tier or backend for this session/agent. |
| `/effort` | Switch reasoning effort for this session/agent when the backend supports it. |
| `/agent` | Switch, inspect, create, or edit agents. |
| `/settings` | Open Assistant settings. |
| `/config` | Alias for `/settings` for Claude Code muscle memory. |
| `/permissions` | Open grant rules, pending grants, denied tools, and audit history. |
| `/tools` | Show enabled host tools, app connectors, MCP tools, and tool collections. |
| `/apps` | Show app connectors available in the workspace. |
| `/skills` | Show installed skills and marketplace skill packs. |
| `/install` | Install a local or marketplace skill/tool/agent package. |
| `/hooks` | Show lifecycle hooks and their source. |
| `/audit` | Show recent Assistant tool calls, grants, app writes, and denied attempts. |
| `/export` | Export the current transcript and tool-call log. |
| `/rewind` | Restore the conversation to an earlier checkpoint; code/app state rollback is separate and must be explicit. |
| `/new` | Create a new named conversation without deleting the current one. |
| `/history` | Open conversation history and checkpoint browser. |

Skill commands:

- `/skill-name args` invokes an installed skill.
- `/pack:skill-name args` disambiguates when two packs provide the same command.
- Disabled skills stay out of the command picker and out of model-visible skill listings.

App commands:

- `/app <id>` opens or focuses an app.
- `/use <app-or-tool>` pins an app connector or tool collection into the current session.
- `/grant <tool-or-app-scope>` opens the permission sheet for a specific connector.
- `/revoke <tool-or-app-scope>` revokes a persisted grant.

The command picker must show source and trust:

- Built-in
- User skill
- Workspace skill
- Marketplace package
- App connector
- MCP server
- Managed policy

Essential commands should exist before marketplace packages depend on the Assistant. Users coming from Claude Code should not have to relearn the basics: clear, compact, resume, context, memory, model, effort, settings/config, permissions, tools, skills, hooks, audit, export, history, and rewind.

## Conversation Persistence

Opening and closing the Assistant pane does not start over. The Assistant resumes the same conversation at the same scroll position, selected agent, loaded prompt set, active tool pins, and pending permission state.

Rules:

- Closing the pane hides the Assistant view. It does not delete or clear the session.
- Reopening the Assistant in the same workspace returns to the active conversation.
- `/new` creates a new conversation and makes it active.
- `/clear` starts a fresh context in the same conversation lineage and leaves the previous transcript resumable.
- `/resume` lists prior conversations for the workspace.
- `/compact` writes a compaction boundary into the transcript. It does not overwrite raw history unless retention settings later prune it.
- If the host quits during a streaming response or tool call, restart shows the interrupted state and marks incomplete work as interrupted rather than pretending it finished.

The active conversation id is workspace-scoped and channel-aware. Multiple windows or panes in the same workspace attach to the same active conversation unless the user explicitly opens another one.

## History And Rewind

Double-tapping `Escape` opens the Assistant history surface. This is the fast path for "go back" without requiring the user to remember `/history` or `/rewind`.

The history surface shows:

- user prompts
- assistant responses
- compaction boundaries
- tool calls
- permission decisions
- model tier switches
- app connector calls
- file/app/code checkpoints
- interrupted turns

Actions:

- Jump conversation view to a prior turn.
- Fork from a prior turn into a new conversation.
- Revert conversation context to a prior turn while leaving files/apps untouched.
- Revert conversation plus host-managed changes when a checkpoint supports rollback.
- Export a selected range.
- Inspect the audit trail for a selected turn.

Checkpoint model:

- Conversation checkpoints are always available.
- Host-managed state checkpoints are available for app connector calls that return changed-resource metadata or reversible operations.
- Code/file checkpoints are available only when the Assistant made the change through host-mediated file tools or a VCS-backed operation the host can prove.
- Terminal commands are not automatically reversible. They can appear in history, but rollback requires an explicit reversible checkpoint or VCS state.
- App connector authors should provide preview and rollback metadata for mutating tools where practical.

`Escape` behavior:

- Single `Escape`: cancel menus, close overlays, or stop current composition.
- Double `Escape` within a short timeout: open history.
- If a tool call or model turn is running, double `Escape` opens history in read-only mode and offers interrupt separately.

Rewind must never silently undo code or app data. The UI must distinguish "conversation only" from "conversation + state rollback" and show the exact files, app documents, panes, or connector resources that will be affected.

## Skill And Tool Marketplace

The marketplace must support four Assistant package component types:

- Skills: prompt workflows, supporting files, optional scripts.
- Agents: named Assistant agents with prompt text, model tier policy, permission posture, and enabled tools.
- Tool collections: grouped host tools, MCP tools, app connectors, and permission hints.
- App connector templates: schemas and instructions for working with apps that expose a known capability family.

These components can ship inside a normal app package or as an Assistant-only package. Paid packages are installed locally after purchase. The hosted service may authorize purchase and update access, but the installed package must be inspectable on disk.

Package metadata:

- package id, version, publisher, source, checksum
- component inventory
- token cost estimate for always-visible descriptions
- default enablement state
- required host version
- required app ids or capability families
- declared scripts or external processes
- requested default grants, if any
- trust label

Install UX:

- Show every component before install.
- Show whether the package adds model-visible instructions.
- Show whether it adds tools that can mutate local state.
- Never auto-grant write permissions during install.
- Let users enable components at user, workspace, or local-workspace scope.

## App-Exposed Capabilities

Apps can expose Assistant-callable tools through a host-mediated connector. The connector is not raw CLI access and not ambient process control.

An app connector declares:

- stable tool name
- description written for the model
- JSON schema for inputs
- JSON schema for outputs
- whether the tool reads, writes, deletes, spawns, or sends network traffic
- data scope: app instance, document, workspace, filesystem path, account, or external service
- grant scope options
- preview/dry-run support
- audit summary template

Runtime rules:

- The host decides whether the Assistant can see a connector in the current context.
- The model only sees tools allowed by current settings and grant state.
- Tool calls go through the host permission broker before reaching the app.
- The app receives the call with its app identity and the grant id.
- The app returns structured output, a user-facing summary, and optional changed-resource metadata.
- Every call writes an audit event.

### CSV Viewer Example

A CSV viewer app with read/write privileges can expose:

- `csv.describe_table`: read-only summary of columns, row count, selection, and detected types.
- `csv.read_range`: read-only cell range.
- `csv.propose_changes`: dry-run mutations with a diff preview.
- `csv.write_range`: write cells in a named file or app document.
- `csv.add_column`: write schema change.
- `csv.save_as`: write a new local file.

Grant flow:

1. User asks: "Normalize the amount column in this spreadsheet."
2. Assistant sees the focused CSV app exposes `csv.describe_table` and `csv.propose_changes`.
3. Read-only calls run if the user has allowed app-state reads for the session.
4. Before `csv.write_range`, the host shows a permission sheet: app identity, file/document, exact tool, proposed scope, and duration.
5. User can choose:
   - Allow once.
   - Allow for this document this session.
   - Always allow this signed app package to let this agent write this document.
   - Deny.
6. The write call includes the grant id. The CSV app applies the change and returns changed ranges.
7. The audit log records the request, grant, input summary, changed ranges, and package identities.

The "always" option is never global by default. It must bind to the narrowest meaningful scope: tool collection, agent, app package identity, document id/path, workspace, and user.

## Permission Model

Assistant permissions are enforced by the host, not by prompt text. This section describes the user-facing behavior of the unified broker above.

Rule types:

- Deny: hide or block the tool.
- Ask: show a prompt before the call.
- Allow: approve without prompting inside the rule scope.

Evaluation order:

1. Managed deny.
2. User/workspace/session deny.
3. Managed ask.
4. User/workspace/session ask.
5. Existing grant.
6. Allow rule.
7. Default posture.

Default postures:

- `review`: read-only host context is allowed; writes ask.
- `plan`: no writes, no app mutations, no terminal input.
- `work`: common low-risk writes can use persisted grants; risky writes ask.
- `locked`: only explicitly allowed tools are visible.

Risk classes:

- Read host state.
- Read app state.
- Read local files.
- Write app document.
- Write local files.
- Send terminal input.
- Spawn process.
- Network request.
- Secret access.
- Install package.
- Delete or destructive change.

Protected actions always ask unless managed policy says deny:

- deleting local files
- changing secrets
- installing packages
- writing outside the workspace
- sending terminal commands that look destructive
- granting "always" permissions

Permission prompts must show:

- who is asking: agent and model backend
- what will run: tool/app connector name
- who receives it: app package identity, MCP server, or host API
- what it can touch: file, document, pane, account, workspace, or app state
- duration choices
- audit destination

## Hooks

Hooks let users and packages observe or control Assistant behavior. They are useful for validation, logging, redaction, policy, and workflow glue.

Events:

- `SessionStart`
- `SessionEnd`
- `UserPromptSubmit`
- `CommandExpansion`
- `PreToolUse`
- `PermissionRequest`
- `PermissionDenied`
- `PostToolUse`
- `ToolUseFailure`
- `PostToolBatch`
- `PreCompact`
- `PostCompact`
- `ConfigChanged`
- `PackageInstalled`
- `AppConnectorAvailable`

Hook transports:

- built-in host callback
- local command
- HTTP endpoint
- prompt hook

Local command and HTTP hooks need their own permissions. A marketplace package cannot silently add an active hook.

## UI Requirements

The Assistant pane uses host UI kit primitives:

- chat transcript with user, assistant, tool, error, and permission rows
- composer with slash command picker
- active agent/model/tool summary
- context drawer for panes, files, apps, and loaded instructions
- permission sheet for tool calls
- settings view
- skill/tool package details view
- audit view

Tool calls should be visible but compact:

- pending
- approved
- denied
- running
- succeeded
- failed

For app writes, show a short diff or changed-resource summary before the user grants permission whenever the connector supports preview.

## Storage

All state lives on disk in channel-aware paths:

- transcripts
- compaction summaries
- history index
- conversation checkpoints
- reversible host-state checkpoints
- installed Assistant packages
- enabled component indexes
- settings
- grants
- audit events
- cached marketplace metadata

Transcripts and audit logs are separate. Deleting a transcript does not delete permission audit history.

## Migration From PGAP Assistant

Phase 1:

- Keep the PGAP Assistant as-is while the host app lands.
- Add the host Assistant behind a feature flag or hidden command.
- Move session storage to the new Assistant store.
- Add host-native chat rendering and streaming.

Phase 2:

- Move pane/app/terminal powers to host tools.
- Replace CLI subprocess tool calls with typed `AssistantEffect`s.
- Add app connector discovery and permission prompts.
- Keep `ask_assistant` as a host service that PGAP apps can call through capability-gated APIs.

Phase 3:

- Retire or rename `apps/assistant/` to `apps/examples/assistant-lite/`.
- Keep coding, writing, and research PGAP agents as examples of normal `ai.query` apps.
- Update app framework docs so third-party agents learn the non-privileged pattern.

## Done When

- A user can open the host Assistant pane and chat with streaming responses.
- `/` opens a searchable command picker with built-ins, skills, app connectors, and installed packages.
- `/settings`, `/permissions`, `/skills`, `/tools`, and `/audit` are real host views.
- Assistant settings support user, workspace, local workspace, and session scope.
- Skills can be invoked manually and auto-selected by description.
- Tool collections and agents can be installed locally and inspected before enablement.
- A running PGAP app can expose a read-only connector and a mutating connector.
- The Assistant can call the read-only connector without write grants.
- A mutating connector prompts before it runs, supports allow-once and persisted narrow grants, and writes an audit event.
- The current PGAP Assistant no longer shells out to the Plexi CLI for pane/app/terminal control.
- HostHarness tests cover command parsing, settings precedence, permission rule order, grant persistence, app connector filtering, and audit writes.
- PlexiUiHarness tests cover the Assistant pane, command picker, permission sheet, settings view, and app-write preview.
