---
title: Run your first agent
description: Connect Claude Code, Codex or Pi to Plexi so the host knows when an agent is working, blocked or done.
order: 2
---

Plexi hosts the coding agents you already use. A coding agent runs in a
terminal pane like any other program. Two small installs make it a Plexi agent:
status hooks, so the host knows what the agent is doing, and a skill, so the
agent knows how to drive Plexi.

You need Plexi [installed](/docs/getting-started) and at least one agent CLI on
your `PATH`: Claude Code, Codex or Pi.

## 1. Install the status hooks

```sh
plexi agent hook install --claude-code
```

Use `--codex` or `--pi` for those agents; the flags combine. The command adds
lifecycle hooks to the agent's own config file. Each hook calls
`plexi agent report`, which tells the host the state of the agent in that pane.

To remove them later:

```sh
plexi agent hook uninstall --claude-code
```

## 2. Teach the agent the CLI

```sh
plexi skill install --agent claude
```

This writes a `SKILL.md` where the agent loads user skills. `--agent` takes
`claude`, `codex`, `pi` or `all`. The skill is the copy compiled into your
binary, so it always describes the commands your install has.

The agent can also learn Plexi without the skill: `plexi --help` and
`plexi <command> --help` cover every command.

## 3. Start the agent in a pane

Split a pane with `⌘D` and start your agent the way you always do:

```sh
claude
```

Or open a named pane from the CLI and leave your focus where it is:

```sh
plexi pane new "claude" -n "worker" --no-focus
```

## 4. Watch its state

Give the agent a task. Then, from any other pane:

```sh
plexi agent status
```

The table lists each pane that has reported agent state: pane ID, agent name,
state and session ID. The states are `working`, `blocked`, `needs_you`, `idle`,
`interrupted`, `failed` and `completed`. Add `--blocked`, `--working` or
`--idle` to filter.

With a few agents running you rarely want the table. You want the one that
needs you:

```sh
plexi agent next
```

This focuses the next agent that is waiting on you, is blocked, or finished
while you were looking elsewhere.

## 5. Answer what it asks

Approvals and agent questions collect in one place:

```sh
plexi needs-you list
```

An agent cannot approve its own request. Permission approvals happen by a
click in the Plexi window, and `plexi needs-you resolve` refuses to approve
one from the CLI. It can still deny.

The same rule covers secrets. A pane Plexi opens for an agent never gets
workspace secrets in its environment, and a secret read from a pane waits for
your click. [Secrets](/docs/secrets) has the details.

## Drive one agent from another

Any pane can be scripted, which is how an agent hands work to another agent:

```sh
plexi pane send worker "Read the brief and implement the fix" --submit
plexi pane capture worker
```

`--submit` presses Enter and exits 0 only once the host confirms the prompt
left the input line.

## Going further

Named agent definitions, `plexi agent spawn`, structured results and teams
are behind the `agent_mesh` experiment, which is off by default:

```sh
plexi experiments enable agent_mesh
```

The [CLI reference](/docs/cli#plexi-agent) documents the whole `plexi agent`
namespace.

## Next

[Build your first plugin](/docs/first-plugin), or have your agent build it.
