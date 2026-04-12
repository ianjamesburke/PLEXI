# Plexi Companion App

**Status:** Design  
**Last updated:** 2026-04-11

---

## Overview

The Plexi Companion App is a native iOS app that provides remote access to Plexi agents running on a paired Mac. It is not a terminal emulator. It is an agent interface — text chat, voice chat, approval management, and notifications — all scoped to specific project directories on the remote machine.

Core capabilities:
- **Text chat** with agents scoped to any `.plexi/` directory on the paired machine
- **Voice chat** via Gemini Live API for real-time bidirectional audio
- **Approval management** with biometric confirmation for high-risk operations
- **Notification feed** for job completions, cost alerts, and agent events
- **Directory navigation** to switch agent scope across projects

The companion app never touches the shell directly. It talks to agents. Agents decide what commands to run.

---

## Pairing & Authentication

### First Connection (Pairing Ceremony)

1. User opens Plexi on Mac, navigates to Settings > Remote Access.
2. Plexi generates:
   - A one-time 6-digit pairing code (expires after 60 seconds)
   - An Ed25519 keypair for this machine
   - A QR code encoding: `{ code, ip, port }`
3. User opens companion app, taps "Connect to Machine."
4. User scans the QR code, or enters the code + IP manually.
5. Companion app sends to Plexi: `{ pairing_code, companion_public_key }`.
6. Plexi verifies the code, stores the companion's public key, responds with its own public key.
7. Both sides now hold each other's public keys. Pairing is complete. The code is burned.
8. Device is registered in `~/.plexi-alpha/paired-devices.json`.

### Subsequent Connections

1. Companion app discovers Plexi via Bonjour/mDNS (local network) or Tailscale (remote).
2. WebSocket connection established with mutual TLS. Both sides verify pinned public keys from the pairing ceremony.
3. Companion app requires Face ID / Touch ID before establishing the session.
4. Plexi sees a verified connection from a paired device with biometric confirmation.
5. Session established. All communication is encrypted end-to-end.

### Device Management

`~/.plexi-alpha/paired-devices.json` stores all paired devices:

```json
[
  {
    "device_name": "Ian's iPhone",
    "public_key": "base64-encoded-ed25519-pubkey",
    "paired_at": "2026-04-11T10:30:00Z",
    "last_connected": "2026-04-11T18:45:00Z"
  }
]
```

- **Revoke access:** Remove the device entry. Its key is no longer trusted.
- **Re-pair:** Run the pairing ceremony again. A new keypair is generated; the old key is invalidated.

---

## Transport Protocol

WebSocket over TLS. Same newline-delimited JSON protocol used by local Plexi apps, extended for remote agent communication.

### Companion -> Plexi

| Type | Fields | Description |
|---|---|---|
| `agent_message` | `directory`, `content` | Send a text message to the agent scoped to `directory` |
| `approval_response` | `approval_id`, `decision` | Respond to a pending approval (`approve` or `deny`) |
| `scope_change` | `directory` | Switch the active directory scope |
| `voice_transcript` | `directory`, `content`, `source` | Transcribed voice input (source: `gemini_live`) |

```json
{"type": "agent_message", "directory": "/Users/ian/projects/brand", "content": "What's the render status?"}
{"type": "approval_response", "approval_id": "apr_001", "decision": "approve"}
{"type": "scope_change", "directory": "/Users/ian/projects/other-brand"}
{"type": "voice_transcript", "directory": "/Users/ian/projects/brand", "content": "Turn the bass down", "source": "gemini_live"}
```

### Plexi -> Companion

| Type | Fields | Description |
|---|---|---|
| `agent_response` | `directory`, `content`, `cost_usd` | Agent's response to a message |
| `approval_request` | `id`, `action`, `agent`, `risk_score`, `context`, `directory` | Agent needs approval for a risky operation |
| `notification` | `title`, `body`, `directory`, `priority` | General event notification |
| `job_status` | `job_id`, `status`, `progress`, `description` | Progress update for a running job |
| `presence_update` | `directory`, `users[]` | Who is active in a directory |

```json
{"type": "agent_response", "directory": "/Users/ian/projects/brand", "content": "Render completed 20 min ago...", "cost_usd": 0.012}
{"type": "approval_request", "id": "apr_001", "action": "git push origin main", "agent": "video-hop", "risk_score": 0.78, "context": "Pushing final render", "directory": "/Users/ian/projects/brand"}
{"type": "notification", "title": "Stills generation complete", "body": "5 images, $0.20", "directory": "/Users/ian/projects/brand", "priority": "normal"}
{"type": "job_status", "job_id": "job_001", "status": "running", "progress": 0.6, "description": "Generating scene 5 still"}
{"type": "presence_update", "directory": "/Users/ian/projects/brand", "users": ["ian", "francisco"]}
```

---

## Voice Interface

Uses Gemini Live API for real-time bidirectional audio streaming. Audio flows between the phone and Gemini; text flows between Gemini and the Plexi agent.

### Architecture

```
Phone Mic -> Gemini Live API -> Transcription -> Plexi Agent
                                                      |
Phone Speaker <- Gemini Live API <- Synthesis <- Agent Response
```

1. User taps the microphone button or says the wake word.
2. Audio streams from the phone to Gemini Live API for real-time transcription.
3. Transcribed text is sent to the Plexi agent as a `voice_transcript` message (same as typing).
4. Agent response text is sent to Gemini Live for speech synthesis.
5. Synthesized speech plays through the phone speaker or headphones.

### Voice-Specific Features

**Briefing mode:** Say "Catch me up." The agent summarizes all pending items across all scoped directories, reads them sequentially, and waits for a response to each before moving on.

**Sidebar conversations:** Tap the sidebar button or say "sidebar" to branch into a sub-conversation that does not appear in the main agent thread. Say "done" to return to the main thread.

**Continuous listening mode:** The agent stays connected. Issue commands without tapping the mic each time. Gemini handles background noise filtering.

**Multi-directory navigation:** Say "Switch to brand campaign" to re-scope the agent. "Go back" pops the scope stack.

### Latency Target

Under 2 seconds from speech end to audio response start. Gemini Live's streaming model makes this achievable for transcription + synthesis. Agent response time is additive and depends on the LLM backend.

---

## Approval Management

When an agent hits an operation with a risk score above the auto-approve threshold, the approval request is forwarded to the companion app.

### Approval Flow

1. Plexi sends an `approval_request` message over the WebSocket.
2. Companion app delivers a push notification with the action description.
3. User taps the notification to open an approval card showing:
   - Action (e.g., `git push origin main`)
   - Agent name
   - Risk score (0.0 - 1.0)
   - Context summary
   - Directory
4. Three buttons: **Approve**, **Deny**, **Show Details**.
5. If the risk score exceeds the biometric threshold (configurable, default `0.92`): the Approve button triggers Face ID / Touch ID. Only after biometric success does the approval go through.
6. Companion app sends an `approval_response` message back to Plexi.

### Timeout

Configurable (default: 5 minutes). If no response within the timeout, the operation is denied and the agent is notified.

### Approval History

A log of all approval requests with:
- Decision (approved / denied / timed out)
- Timestamp
- Agent name
- Action
- Risk score
- Directory

Filterable by directory, agent, risk score, and decision.

---

## Directory Navigation

The companion app shows a list of all directories on the paired machine that contain a `.plexi/` subdirectory. Plexi scans and provides this list over the WebSocket on connection.

Each directory entry shows:
- **Name and path**
- **Active agents** and their status (idle, running, waiting for approval)
- **Presence** (other users active in this directory)
- **Pending approvals count**
- **Recent activity summary** (last 3 events)

Tapping a directory scopes the chat and voice interface to that directory. All subsequent messages are routed to the agent in that scope until the user switches.

---

## Notification Feed

A chronological feed of events aggregated across all directories:

| Event Type | Example |
|---|---|
| Job completion | "Stills generation complete — 5 images, $0.20" |
| Approval pending | "video-hop wants to run git push origin main" |
| Approval resolved | "Approved: git push origin main" |
| Cost alert | "Daily spend: $4.50 (80% of $5.00 budget)" |
| Presence change | "francisco joined /projects/brand" |
| Error alert | "Agent failed: tool error in generate-still.py" |

Each notification is actionable. Tapping it scopes the app to the relevant directory and opens the appropriate view (chat, approval card, or job detail).

---

## Settings

### Per-Machine Settings

| Setting | Type | Default | Description |
|---|---|---|---|
| Connection method | `local` / `tailscale` | `local` | How to discover and connect to this machine |
| Auto-connect on launch | `bool` | `true` | Connect to this machine when the app opens |
| Notification level | `all` / `approvals` / `none` | `all` | Which events generate push notifications |
| Biometric threshold | `f64` (0.0 - 1.0) | `0.92` | Risk score above which Face ID is required to approve |
| Approval timeout | `u32` (seconds) | `300` | Time before unanswered approvals are auto-denied |
| Session timeout | `u32` (seconds) | `1800` | Inactive session expiry (re-auth with biometric required) |

### Per-Directory Overrides

These override machine-level defaults for a specific directory:

| Setting | Type | Description |
|---|---|---|
| Notification level | `all` / `approvals` / `none` | Override notification level for this directory |
| Voice commands allowed | `bool` | Whether voice input is accepted for this directory |

---

## Security Model

**No shell access.** The companion app talks to agents, not to zsh. The agent decides what commands to run based on its scoping rules and capability set.

**No raw file access.** The companion app cannot read or write files directly. All file operations go through the agent, which is subject to the existing capability and approval system.

**Biometric is local.** Face ID / Touch ID validation happens on the phone. The phone sends a signed assertion to Plexi. Plexi trusts the biometric claim from a paired device because the device identity was established during the pairing ceremony (mutual key exchange).

**Key rotation.** Keypairs can be rotated via a re-pairing ceremony. Old keys are immediately invalidated. There is no key migration — re-pairing generates fresh keys on both sides.

**Session timeout.** Inactive sessions expire after a configurable period (default 30 minutes). Resuming requires biometric re-authentication.

**No API keys in transit.** The companion app never sees API keys, model tokens, or secrets. It talks to the agent. The agent retrieves keys from the local secrets manager on the Mac.

**Transport encryption.** All WebSocket traffic is TLS-encrypted with certificate pinning. The pinned keys are the Ed25519 public keys exchanged during pairing — no CA trust chain involved.

---

## Implementation Phases

### Phase 1: Text Chat + Approvals

- Pairing ceremony (QR code + manual code entry)
- WebSocket connection with Ed25519 keypair auth
- Text-based agent chat scoped to one directory
- Approval request forwarding with approve/deny
- Face ID / Touch ID for high-risk approvals (risk > biometric threshold)
- Push notifications for approval requests

### Phase 2: Voice + Multi-Directory

- Gemini Live API integration for voice chat
- Directory tree navigation and scope switching
- Multi-directory voice navigation ("switch to X", "go back")
- Briefing mode ("catch me up")
- Sidebar conversations

### Phase 3: Notifications + Dashboard

- Full notification feed across all directories
- Job status monitoring with progress indicators
- Cost dashboard (session and daily spend)
- Presence indicators per directory
- Approval history with filtering

### Phase 4: Multi-Machine

- Pair multiple machines from one companion app
- Machine switcher in the app
- Cross-machine notification aggregation
- Unified approval queue across all paired machines

---

## Tech Stack (iOS)

| Component | Technology |
|---|---|
| UI framework | SwiftUI |
| Networking | Network.framework (WebSocket + TLS) |
| Cryptography | CryptoKit (Ed25519 keypairs, signing, verification) |
| Biometrics | LocalAuthentication (Face ID / Touch ID) |
| Push notifications | UserNotifications |
| Voice | Google AI Swift SDK (Gemini Live API) |
| QR scanning | AVFoundation |
| Local storage | SwiftData (paired devices, approval history, settings) |

No backend server. All communication is direct device-to-device over the local network or Tailscale.
