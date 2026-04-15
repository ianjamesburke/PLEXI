# Agent Mode

**Status:** Design  
**Last updated:** 2026-04-11

---

## Overview

Agent mode is the terminal's second mode. Shell mode talks to zsh. Agent mode talks to an LLM. Same pane, same output area.

Press `/` at an empty prompt to switch. The agent is scoped to the current directory — it knows about installed apps, agents, project files, and secrets. It can run shell commands on your behalf (they appear as if you typed them), read and write files, interact with apps, and spawn background jobs.

When the agent runs a command, it's visible in the terminal output. No hidden actions.

---

## Mode Switching

### Entering Agent Mode

Press `/` when the prompt is empty. This is the only trigger.

**Why `/` doesn't conflict with shell input:** `/` only triggers agent mode when it's the FIRST character on an EMPTY prompt line. If you're mid-command (`cd /usr/...`), the prompt isn't empty — `c` was the first character. Paths typed from scratch (`/usr/bin/foo`) are an edge case worth noting: the user would need to type a space first or use `cd`. In practice, standalone absolute paths at the prompt are rare — they require execute permission and most workflows use a command prefix.

### Exiting Agent Mode

- **Escape** — returns to shell mode immediately, discarding any in-progress input
- **Agent finishes responding** — automatically returns to shell mode
- **Agent explicitly exits** — when the agent determines the task is complete

On exit, the pane border reverts, the prompt prefix restores, and keystrokes route to the PTY again.

### Visual Indicators

When agent mode is active:

| Element | Change |
|---|---|
| Pane border | Shifts from neutral gray to a soft blue accent |
| Prompt prefix | Changes character or color (distinct from shell `$` / `>`) |
| Status strip | Thin bar below the prompt showing: directory scope, active jobs count, agent's last action |
| Agent text | Rendered with a left-border accent line — same font, same area, visually distinct from shell output |

All indicators revert on exit.

---

## Input Handling

### Shell Mode (Default)

All keystrokes go directly to the PTY. Plexi renders terminal output normally. No interception except Plexi's own shortcuts (Cmd+K, Cmd+H/J/K/L, etc.).

### Agent Mode

Keystrokes go to Plexi's internal input buffer, NOT to the PTY. Plexi renders the input buffer at the same position as the shell prompt.

| Key | Behavior |
|---|---|
| Characters | Appended to input buffer |
| Enter | Submit input buffer to agent |
| Shift+Enter | Insert newline (multi-line messages) |
| Backspace | Delete character in input buffer |
| Left/Right | Move cursor within input buffer |
| Up/Down | Scroll through conversation history |
| Tab | Autocomplete slash commands |
| Escape | Exit agent mode |

---

## Slash Commands

When in agent mode, input prefixed with `/` is handled by Plexi directly (not sent to the LLM):

| Command | Description |
|---|---|
| `/status` | Show status of all running jobs |
| `/cost` | Show cost summary (session, daily, project) |
| `/jobs` | List active and recent background jobs |
| `/approve <id>` | Approve a pending operation |
| `/deny <id>` | Deny a pending operation |
| `/history` | Show recent conversation in this directory |
| `/clear` | Clear conversation context (start fresh) |
| `/scope` | Show current directory scope and available apps/agents |

Tab on `/` shows available commands with descriptions.

---

## Agent Context Loading

When agent mode is activated, context is assembled from these sources (in load order):

1. **Directory-specific agent config** — `.plexi/agents/terminal/system.md` (project-specific instructions)
2. **Agent memory** — `.plexi/agents/terminal/memory/` (accumulated context from past conversations in this directory)
3. **App capabilities** — `.plexi/apps/*/agents.md` from all installed apps (available tools)
4. **App manifests** — `.plexi/apps/*/manifest.toml` (descriptions, permissions)
5. **System-level equivalents** — `~/.plexi-alpha/agents/`, `~/.plexi-alpha/apps/`
6. **Project files** — filesystem access within the scoped directory

Context loads once on activation. If you `cd` to a different directory and re-enter agent mode, context reloads for the new directory.

---

## Agent Execution

When the user submits a message:

1. Message goes to the configured LLM (model from directory-level or system-level config).
2. Agent processes the message with full context.
3. Agent can perform any of these actions:

| Action | How it works |
|---|---|
| **Text response** | Rendered inline in terminal output with accent styling |
| **Shell command** | Sent to PTY stdin as if the user typed it — output appears normally |
| **App interaction** | Events sent to app stdin via the draw protocol |
| **File read/write** | Filesystem access within the scoped directory |
| **Background job** | Spawned as a subprocess, tracked by Plexi |
| **Approval request** | Surfaced to user in terminal or companion app |

Agent responses and shell command outputs interleave in the terminal. The visual distinction (accent border for agent text, normal rendering for shell output) makes the source clear.

---

## Background Jobs

The agent can spawn background work with dependency graphs:

```
User: generate stills for scenes 4-8, then render the video when done

Agent: Starting 2 jobs:
  [job_001] Generating stills for scenes 4-8 (autonomous)
  [job_002] Render video (queued, depends on job_001)
```

Jobs run as subprocesses. Plexi tracks status and dependencies.

### Job Status Strip

When jobs are active, a status strip appears at the bottom of the terminal:

```
[2 jobs: 1 running, 1 queued] scene 5 still ██████░░░░ 60%
```

### Attention Levels

Each job gets an attention level determined by comparing the agent's trust score against the operation's risk score — both continuous floats (0.0-1.0):

| Level | Behavior |
|---|---|
| `autonomous` | Run to completion, log results silently |
| `notify` | Run to completion, show notification when done |
| `gate` | Pause before executing, show the user what's about to happen, wait for approval |

The thresholds between levels are configured in `config.toml` (see Configuration).

---

## Conversation Persistence

Conversations are stored per-directory:

```
.plexi/agents/terminal/
  conversations/
    2026-04-11_143022.json       # one file per session
  memory/
    learnings.md                 # accumulated context (written by improvement agent)
  system.md                      # project-specific agent instructions (optional)
```

### Conversation File Format

```json
{
  "id": "conv_2026-04-11_143022",
  "directory": "/Users/ian/projects/brand",
  "started": "2026-04-11T14:30:22Z",
  "messages": [
    {
      "role": "user",
      "content": "What's the render status?",
      "timestamp": "2026-04-11T14:30:22Z"
    },
    {
      "role": "assistant",
      "content": "Job completed...",
      "timestamp": "2026-04-11T14:30:25Z",
      "cost_usd": 0.012
    }
  ],
  "total_cost_usd": 0.045
}
```

---

## Trust and Risk System

Every operation the agent performs is scored before execution:

```json
{
  "operation": "write_file",
  "path": "stills/scene_05.png",
  "risk_score": 0.25,
  "agent_trust": 0.72,
  "prediction_confidence": 0.91,
  "decision": "auto_approve"
}
```

### Decision Logic

- `prediction_confidence > auto_approve_threshold` — execute without asking
- `prediction_confidence < auto_deny_threshold` — block and notify user
- Otherwise — surface approval request (terminal or companion app)

### Default Risk Scores

These are starting values. The system self-tunes based on user approval/denial patterns:

| Operation | Risk Score |
|---|---|
| `read_file` | ~0.05 |
| `write_file` (in project dir) | ~0.25 |
| `run_script` (approved list) | ~0.35 |
| `delete_file` | ~0.65 |
| `git_push` | ~0.75 |
| `run_arbitrary_command` | ~0.80 |
| `git_force_push` | ~0.95 |

### Forbidden Operations (Risk = 1.0, Always Denied)

- Modify files outside scoped directory
- Modify system files
- Disable logging
- Modify own trust score
- Access other users' directories

---

## Remote Access Integration

The terminal agent is the same agent the companion app talks to. Messages arriving from the companion app (via WebSocket) enter the same processing pipeline as local `/` input. The agent doesn't know whether input came from the keyboard, voice, or a remote device.

Approval requests can surface to:

1. The local terminal (inline prompt)
2. The companion app (push notification with approve/deny buttons)
3. Both (configurable per-directory or system-level)

---

## Configuration

### System-Level (`~/.plexi-alpha/config.toml`)

```toml
[agent]
default_model = "claude-sonnet-4-6"
auto_approve_threshold = 0.88
auto_deny_threshold = 0.15
biometric_threshold = 0.92       # risk above this requires Face ID (companion app)
conversation_history_limit = 50  # messages kept in context
```

### Directory-Level (`.plexi/agents/terminal/config.yaml`)

```yaml
model: claude-sonnet-4-6         # override model for this directory
system_prompt: system.md          # relative path to system prompt
trust_overrides:
  write_file: 0.85               # higher trust for writes in this project
  delete_file: 0.40              # lower trust for deletes
```

Directory-level config overrides system-level where both are set. Unset fields fall through to system-level values.

---

## Implementation Notes

### Architecture

The agent backend is a separate concern from the terminal UI. Two components communicate via an internal channel (`mpsc` or similar):

1. **Terminal pane** — owns mode switching, input buffer rendering, visual indicators, and output rendering
2. **Agent backend** — owns LLM calls, tool execution, context loading, job tracking, and conversation persistence

### Rust-Side Changes

**Terminal pane (`src/terminal_pane.rs` or equivalent):**
- Add `AgentMode` variant to the pane's mode enum
- When `AgentMode` is active, keystrokes write to an internal `InputBuffer` instead of the PTY
- Render the input buffer at the prompt position
- Render agent responses with accent border styling
- Show/hide status strip based on active job count

**Input routing:**
- On keypress: check mode. Shell mode → write to PTY. Agent mode → write to input buffer.
- On Enter in agent mode: send input buffer contents to agent backend via channel, clear buffer.
- On `/` when prompt is empty and mode is Shell: switch to Agent mode.

**Agent backend (new module):**
- Spawn as an async task — never blocks the render loop
- Receives user messages via channel from the terminal pane
- Sends responses back via channel (text chunks for streaming, command executions, etc.)
- Manages conversation state and persistence
- Manages the job dependency graph

**Shell command injection:**
- When the agent runs a command: write the command bytes to PTY stdin
- The command appears in the terminal as if the user typed it
- Agent backend waits for output by reading from PTY stdout (or a sentinel marker)

**Job tracker:**
- In-memory job graph: `HashMap<JobId, Job>` with status, dependencies, attention level
- Jobs are subprocesses spawned by the agent backend
- Status updates propagate to the terminal pane for the status strip
