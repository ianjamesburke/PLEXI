# Voice Agent Design

## Summary

Add voice interaction to Plexi: double-tap Fn to start listening, VAD-gated Whisper transcription, a daemon voice agent that interprets commands via Plexi IQ (Gemini Flash through OpenRouter), executes actions (spawn panes, pipe commands to apps), and responds via TTS (direct Gemini API). Demo target: open a calendar app, add/update events, close it — entirely by voice.

## Architecture

```
Double-tap Fn → Host listening state ON → visual indicator (dot)
         ↓
   Mic capture (existing audio_capture infra)
         ↓
   VAD (silero-vad, ~1ms/frame) — gates Whisper
         ↓
   Speech segment → Whisper tiny → text
         ↓
   Voice Agent daemon pane (respond() event)
         ↓
   emit.ai_query(tier="low") — intent parsing
         ↓
   Actions: spawn_pane / pipe_send / emit.speak()
```

### Key Principles

- Host owns all audio I/O — agent never touches mic or speaker directly
- Voice always routes to the dedicated voice agent daemon, not focused pane
- Agent uses existing Plexi IQ broker (OpenRouter, tiered models) — no new AI plumbing
- TTS is a host primitive (`emit.speak(text)`) — any app can request speech
- Nothing in this design prevents future sandboxing/capability-gating at the host layer

## New Host Primitives

### 1. Listening State

- Global boolean toggle on HostModel: `listening: bool`
- Activated by double-tap Fn (configurable in keybindings)
- Visual indicator: dot in top-right corner of the active context, color-coded:
  - Off (default) — no dot
  - Listening (VAD idle) — static dot
  - Hearing speech (VAD active) — pulsing dot
  - Processing (Whisper/LLM running) — spinning/color-shift dot
- Host owns mic capture lifecycle — opens on listen-start, closes on listen-stop

### 2. VAD + Transcription Pipeline

- VAD: silero-vad (ONNX, ~1MB) or webrtcvad — runs per audio frame
- When VAD detects speech: buffer PCM
- When VAD detects 500ms silence after speech: ship buffer to Whisper
- Whisper: `whisper.cpp` with `tiny` model (75MB), called as subprocess or via C FFI
- Output: transcribed text string delivered to voice agent as `respond()` event
- Processing state emitted immediately when Whisper starts (for UI indicator)

### 3. TTS Primitive — `emit.speak(text)`

- New PGAP command: `{"cmd": "speak", "text": "Got it"}`
- Host routes to Gemini Flash TTS (direct Google API, not OpenRouter)
- Plays audio on default output device
- Any app can call `emit.speak()` — not voice-agent-exclusive
- Requires `audio.speak` capability in manifest

### 4. Voice Agent Daemon Pane

- Manifest type: `daemon` (new pane type — always running, not tiled, not focusable normally)
- Summoned as full-screen overlay via global hotkey (Cmd+Shift+Space or configurable)
- Shows chat-style history of voice interactions (what it heard, what it did)
- Dismissed by same hotkey or Escape
- Agent uses `ai_query(tier="low")` with system prompt defining its personality/capabilities
- System prompt loaded from voice config file

### Voice Config

`~/.plexi-alpha/voice.toml`:
```toml
[voice]
enabled = true
hotkey = "fn+fn"           # double-tap Fn
overlay_hotkey = "cmd+shift+space"

[voice.agent]
tier = "low"               # which IQ tier for intent parsing
system_prompt = "You are a concise voice assistant. Respond briefly. For actions, execute silently and confirm with 1-3 words."

[voice.tts]
provider = "google"        # direct Gemini API
model = "gemini-2.5-flash"
api_key_env = "GOOGLE_API_KEY"
```

Per-workspace override: `.plexi/voice.toml` (same structure, merged on top).

## Demo: PAM Calendar

A minimal calendar Plexi app that:
- Stores events in-memory (JSON array)
- Renders a month/week view with vim-style keybinds (j/k navigate, a to add)
- Accepts pipe commands: `add_event`, `update_event`, `delete_event`, `close`
- Responds to pipe commands with confirmation JSON

Demo script:
1. User double-taps Fn (listening active)
2. "Open my calendar" → agent calls spawn_pane("calendar")
3. "Add a baseball game Friday at 7pm" → agent pipes `add_event` to calendar
4. "Actually make that 6pm" → agent pipes `update_event` to calendar
5. "Close the calendar" → agent pipes `close` or host closes pane
6. Agent says "Got it" after each action via TTS

## Foot Guns Addressed

| Risk | Mitigation |
|------|-----------|
| Continuous Whisper CPU burn | VAD gates transcription — Whisper only runs on speech segments |
| Wake word unreliability | Skipped for MVP — double-tap Fn is the trigger |
| Voice routes to wrong pane | Dedicated voice_target (daemon pane), independent of focus |
| No feedback during latency | Processing state indicator (dot animation) fires immediately |
| TTS double-hop latency | Direct Gemini API, not OpenRouter |
| Onboarding blocks usage | Sensible defaults in voice.toml, no setup required |

## Future (Not MVP)

- Configurable wake word via keyword spotter model (Porcupine/OpenWakeWord)
- Onboarding flow to set name, personality, wake phrase
- Apps leasing voice input from host (capability: `voice.listen`)
- Workspace sandboxing — agent can't escape cwd
- Conversation memory across sessions
- Multiple voice agents (per-workspace personalities)
