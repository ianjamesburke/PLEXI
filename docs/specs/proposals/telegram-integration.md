# Telegram App

**Status:** Draft  
**Last updated:** 2026-04-11

---

## 1. Overview

The Telegram app bridges Telegram messages to the Plexi terminal agent. It enables remote agent access, approval flows, and notifications via a Telegram bot.

The app is a standard Plexi out-of-process app (Python, stdin/stdout JSON protocol) with one key difference: it maintains a persistent network connection to Telegram's API via long-polling. It runs as a long-lived process inside Plexi -- Plexi must be open for the bot to be active. There is no background daemon.

When someone messages the bot, the message routes to the terminal agent in the directory where the Telegram app is installed. The agent processes it in that directory's scope, and the response goes back to Telegram.

---

## 2. Architecture

Two message flows, one persistent connection.

### Inbound (Telegram --> Plexi)

1. User sends a message to the Telegram bot
2. App receives the message via `python-telegram-bot` long-polling
3. App validates: is this chat ID registered? Is this user authorized?
4. If the message is a bot command (`/chatid`, `/ping`), handle it directly -- do not forward to the agent
5. App writes a structured message to `.plexi/agents/terminal/inbox/`
6. Terminal agent picks up the message, processes it in the scoped directory
7. Agent writes response to `.plexi/agents/terminal/outbox/`
8. App picks up the response, formats it, sends to Telegram
   - Split messages exceeding 4096 characters
   - Attempt Markdown formatting with plain text fallback
   - Send typing indicator while agent is processing

### Outbound (Plexi --> Telegram)

1. Agent or job completes and writes a notification to the outbox
2. Telegram app watches the outbox directory (polling or filesystem events)
3. App formats the notification and sends it to the registered Telegram chat

### Connection Model

Long-polling, not webhooks. No public URL needed, no SSL certificate, no port forwarding. The `python-telegram-bot` library handles reconnection on network drops.

```
Telegram API  <--long-poll-->  telegram_app.py  <--stdin/stdout-->  Plexi
                                     |
                                     v
                          .plexi/agents/terminal/
                              inbox/    outbox/
```

---

## 3. Security Model

### Chat Registration

Only messages from registered chat IDs are processed. All other messages are silently dropped (no error response to unregistered users -- don't leak that the bot is active).

Registration is local: the user adds their Telegram chat ID to the app's secrets via Plexi's secrets manager (`TELEGRAM_ALLOWED_CHAT_IDS`).

### User Authentication

The bot token + chat ID pair identifies the conversation. For multi-user bots, each Telegram user ID is checked against the allow list. Messages from unregistered users in a registered group are dropped.

### Scope Enforcement

The Telegram app is scoped to its install directory. The agent it talks to can only access files in that directory. The scope is determined by where the app is installed:

| Install location | Agent scope |
|---|---|
| `~/projects/brand-campaign/.plexi/apps/telegram/` | `~/projects/brand-campaign/` only |
| `~/.plexi/apps/telegram/` | User home (broad access) |

Installing in a subdirectory limits scope. Installing at root gives root access. Choose deliberately.

### Approval Forwarding

When the agent hits a dangerous operation (risk score above threshold), the approval request is forwarded to Telegram as an inline keyboard with Approve / Deny / Show Diff buttons. The agent blocks until the user responds.

### No Raw Command Execution

The Telegram user talks to the agent, not the shell. The agent decides what commands to run, subject to the trust and risk system. There is no passthrough to `bash`.

---

## 4. Manifest

```toml
[app]
id = "telegram"
name = "Telegram"
entry = "telegram_app.py"
version = "0.1.0"
description = "Remote agent access via Telegram bot"

[capabilities]
filesystem = "read_write"    # read/write agent inbox/outbox files
terminal_write = false       # doesn't directly run terminal commands
network = true               # connects to Telegram API
intelligence = "none"        # talks to agent, not LLM directly

[secrets]
required = ["TELEGRAM_BOT_TOKEN"]
optional = ["TELEGRAM_ALLOWED_CHAT_IDS"]  # comma-separated, e.g. "tg:123,tg:456"

[settings]
auto_start = { type = "boolean", default = true }
notification_level = { type = "select", options = ["all", "approvals_only", "none"], default = "all" }
```

---

## 5. Setup Flow

1. Create a bot via @BotFather on Telegram. Copy the bot token.
2. Install the Telegram app to the desired directory's `.plexi/apps/telegram/`.
3. Store the bot token via Plexi secrets manager: `TELEGRAM_BOT_TOKEN`.
4. Open Plexi. The Telegram app auto-starts if `auto_start = true`.
5. Message the bot `/chatid`. It responds with `tg:<your_chat_id>`.
6. Store the chat ID: `TELEGRAM_ALLOWED_CHAT_IDS = "tg:123456789"`.
7. Message the bot again. Messages now route to the terminal agent.

**Verification:** Send `/ping`. The bot responds with its status, uptime, and the scoped directory path.

---

## 6. Message Protocol

### Inbound Message

Written to `.plexi/agents/terminal/inbox/<id>.json`:

```json
{
  "id": "msg_001",
  "source": "telegram",
  "chat_id": "tg:123456789",
  "sender_name": "Ian",
  "sender_id": "12345",
  "content": "What's the status of the brand campaign render?",
  "timestamp": "2026-04-11T14:30:00Z",
  "reply_to_id": null,
  "thread_id": null,
  "attachments": []
}
```

### Outbound Response

Written to `.plexi/agents/terminal/outbox/<id>.json`:

```json
{
  "id": "resp_001",
  "in_reply_to": "msg_001",
  "destination": "tg:123456789",
  "content": "Job completed 20 min ago. Final render at output/final.mp4.",
  "timestamp": "2026-04-11T14:30:05Z"
}
```

### Approval Request

Written to outbox, forwarded to Telegram as an inline keyboard:

```json
{
  "id": "approval_001",
  "type": "approval_request",
  "destination": "tg:123456789",
  "action": "git push origin main",
  "agent": "video-hop",
  "risk_score": 0.78,
  "context": "Pushing final approved render and manifest",
  "options": ["approve", "deny", "show_diff"]
}
```

Rendered in Telegram as:

```
Agent "video-hop" wants to run:
  git push origin main

Risk: 0.78 | Pushing final approved render and manifest

[Approve]  [Deny]  [Show Diff]
```

### Attachment Handling

| Telegram type | Behavior |
|---|---|
| Photo | Download largest size (`photos[-1]`), save to `input/` |
| Video | Download, save to `input/` |
| Audio / Voice | Download, save to `input/` |
| Document | Download with original filename, save to `input/` |
| Sticker | Convert to emoji in message text |

Files are saved to the project's `input/` directory with the format `tg_<file_id>_<original_name>`. The attachment path is included in the inbox message's `attachments` array.

---

## 7. Bot Commands

Handled directly by the app. Never forwarded to the agent.

| Command | Response |
|---|---|
| `/chatid` | Returns `tg:<chat_id>` for registration |
| `/ping` | Returns bot status, uptime, scoped directory |
| `/help` | Lists available commands |

---

## 8. Implementation Details

### Library

`python-telegram-bot` (async, well-maintained, closest Python equivalent to grammy).

### Long-Polling

No webhook, no public URL. The library manages the polling loop and reconnection. On network failure: log the error, let the library retry. Never crash.

### Message Splitting

Telegram enforces a 4096 character limit per message. Split on paragraph boundaries when possible, fall back to hard split at 4096. Never truncate -- the user needs the full response.

### Markdown Formatting

Send with `parse_mode="MarkdownV2"`. Wrap every send in try/except -- Telegram's Markdown parser fails on unescaped special characters. On failure, retry with plain text (no `parse_mode`).

### Typing Indicator

Send `sendChatAction("typing")` when the agent starts processing. Fire-and-forget -- if the API call fails, ignore it.

### Thread Support

Telegram forum topics use `message_thread_id`. Forward this value in the inbox message's `thread_id` field. When sending responses, include the thread ID to keep the conversation in the correct topic.

### File Downloads

Telegram file download URLs use the bot token: `https://api.telegram.org/file/bot{TOKEN}/{file_path}`. These URLs expire. Download immediately on message receipt, save to `input/`, then write the inbox message.

### Error Handling

- Network errors: log and let the library reconnect
- API errors on send: retry once, then log and skip
- File download errors: write a text placeholder in the attachment field (`[File download failed: <filename>]`)
- Never crash the process. Log everything with enough context to debug.

---

## 9. Hosting Models

### Single-User (Default)

One bot token, one chat ID. Only you can talk to the agent. This is the expected setup for personal use.

### Multi-User (Team)

One bot token, multiple chat IDs in the allow list. All users' messages go to the same directory agent. Everyone shares the same project context.

Use case: a team shares a project bot that can check build status, deploy, or answer questions about the codebase.

### Public (Hosted Service)

One bot token, open to any user. Each new user gets an isolated subdirectory initialized from a template:

```
hosted-service/
  .plexi/
    apps/telegram/
  template/               <-- base project files + agent config
  users/
    tg_123456789/          <-- initialized on first message
      .plexi/              <-- own workspace, own agent memory
      ...
    tg_987654321/
      .plexi/
      ...
```

On first message from a new user:
1. Copy `template/` to `users/tg_{chat_id}/`
2. Initialize a fresh agent context in the new directory
3. Scope all subsequent messages from that user to their directory

Budget enforcement: each user directory has spend limits in its `.plexi/` config. When budget is exhausted, the bot responds with "Budget exceeded" and stops processing until refilled.

---

## 10. Update Model

When the Telegram app is distributed (someone else installs your bot on their machine):

| Component | Ownership | Update mechanism |
|---|---|---|
| App code (`telegram_app.py`) | Distributed | Plexi app update system (GitHub release or registry) |
| Bot token | User's own | Their own @BotFather bot |
| Agent system prompt (`system.md`) | User data | Not touched by app updates |
| Agent memory (`memory/`) | User data | Not touched by app updates |
| Template directory | Publisher | New version published; existing users opt-in to merge |

App code updates and agent prompt updates are decoupled. Updating the app never overwrites the user's agent configuration.

---

## 11. Gotchas (Lessons from NanoClaw)

1. **Markdown parsing fails silently.** Always wrap sends in try/except with a plain text fallback. Telegram's MarkdownV2 parser chokes on unescaped `_`, `*`, `[`, `]`, `(`, `)`, `~`, `` ` ``, `>`, `#`, `+`, `-`, `=`, `|`, `{`, `}`, `.`, `!` characters.

2. **4096 character limit.** Split messages. Don't truncate. Users need the full response.

3. **Photo array sizes.** Telegram sends multiple resolutions. Always use `photos[-1]` (largest).

4. **File download URLs expire.** Download immediately on receipt. Don't store the URL for later.

5. **Bot commands must be filtered.** `/chatid`, `/ping`, `/help` are handled by the app, never forwarded to the agent.

6. **Thread ID support.** Forum topics use `message_thread_id`. Forward it to maintain conversation threading. Missing this causes replies to land in the wrong topic.

7. **Typing indicators fail silently.** Send them, but don't let failures block message processing.

8. **Long-polling vs webhooks.** Long-polling is simpler (no public URL, no SSL cert). Use it. Webhooks only matter at thousands of concurrent users -- irrelevant for Plexi's use case.

9. **Bot token in secrets.** Read from Plexi's secrets manager, never hardcoded. Auto-enable if token is present, skip gracefully if not.

10. **Reconnection.** Network drops happen. The `python-telegram-bot` library handles reconnection automatically, but log the event so the user knows it happened.
