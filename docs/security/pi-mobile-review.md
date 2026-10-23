# `pi-mobile` Security Review (`0.4.0`)

Static source audit of the open-source [`pi-mobile`](https://github.com/jkeskikangas/pi-mobile) Pi extension package and its bundled companion package `pi-speech` before any installation on Ian's Pi coordinator (`~/.pi/agent/`).

---

## Audited Artifact Provenance

| Field | `pi-mobile` | `pi-speech` (direct runtime dependency) |
|---|---|---|
| **npm package** | `pi-mobile@0.4.0` | `pi-speech@0.1.0` |
| **Upstream repository** | `https://github.com/jkeskikangas/pi-mobile` | `https://github.com/jkeskikangas/pi-mobile` (`packages/pi-speech`) |
| **Git commit (`gitHead` / `HEAD`)** | `938d53dd386b611e6a5920ac65c8ef30d8a9c447` | `938d53dd386b611e6a5920ac65c8ef30d8a9c447` |
| **Tarball SHA-1 (`shasum`)** | `3634961a7ba79e64fdd2f08c88ad6e7eaa7188d2` | `a8ccfd863705d1403941ec562bbc76e18cedf439` |
| **Tarball SHA-256** | `b9c6b512201fd1c3b347524b93781497d74dab02a27ecb5728931cde8e453c97` | `f12afa0629b8d32fc5029946c827d84348041cb8fc21581a916d49a6b8ca2881` |
| **npm `integrity`** | `sha512-oDQ6zsvrNpEyzS8V0xRXBBlUmPuIPLJEywD+gqpwU2NDn69SZ3t+JtsdA/fGikA1QBE5bhNM8H0OqGcozfaWgg==` | `sha512-KfDXpczjsl+KG673B+yh95U7yfATLNWadaSja7lYiv+LXYXqfFlJe0oPCjiDwGu2lPHtnc9LOlwPV219GveX8w==` |
| **License** | MIT (`package.json:10`) | MIT (`packages/pi-speech/package.json:10`) |

All `file:line` references below are anchored to `pi-mobile@0.4.0` (`938d53dd386b611e6a5920ac65c8ef30d8a9c447`) and `pi-speech@0.1.0`.

---

## Executive Summary & Recommendation

**Recommendation: DO NOT install `pi-mobile@0.4.0` as-is (`pi install npm:pi-mobile@0.4.0`); only install with the pinned-config + local-patch guardrails in [Approved Install & Tailscale Runbook](#approved-install--tailscale-runbook) (or keep disabled in favour of Plexi's native relay).**

`pi-mobile` is a clean, non-malicious codebase with zero telemetry, strict loopback HTTP binding (`127.0.0.1:7878`), Tailscale-only HTTPS exposure (`tailscale serve`, never public Funnel), and end-to-end `tweetnacl` XSalsa20-Poly1305 (`secretbox`) encryption between the hub and paired phones. It **cannot** bypass Plexi's stint `0788` desktop click-only broker for permission grants (`ApprovalClick`, `PermissionChange`), `Integrity` alerts, `Keychain` recovery, or `plexi secret get`.

However, installing `pi-mobile@0.4.0` globally into `~/.pi/agent/` **as-is** introduces four operational and security hazards on Ian's machine:

1. **Machine-wide session sharing (`config.enabled = true`):** Running `/remote` once writes `"enabled": true` to `~/.pi/agent/pi-mobile/config.json` (`src/index.ts:278-286`). Every running and future Pi process watches that file every 1.5 s (`src/index.ts:166-171`) and automatically attaches itself to the hub (`src/index.ts:148-156`) unless started with `--no-extensions` or manually opted out.
2. **Phone-answerable `NeedsYouKind::Question` via `ctx.ui.select` monkey-patching:** `pi-mobile` monkey-patches `ctx.ui.select`, `ctx.ui.confirm`, and `ctx.ui.input` inside the Pi extension host (`src/bridge.ts:588-605`). When `@plexi/pi` (`packages/pi-plexi`) runs `plexi_needs_you_ask` with `prompt_in_pi_ui: true`, it passes the host-minted `ui_token` (`PLEXI_PI_UI_ANSWER_TOKEN`) when `ctx.ui.select` resolves (`packages/pi-plexi/src/tools.ts`). A paired `pi-mobile` phone can therefore answer `Question` items via the Pi TUI bridge, whereas Plexi's own phone relay (`ResolveSurface::Phone` in `PermissionMonitor::answer_needs_you_from`) is read-only.
3. **Unconditional `pi-speech` loading & `/voice` command collision:** `src/index.ts:88-98` unconditionally registers `/dictate` and `/voice` from `pi-speech@0.1.0`, colliding with Ian's configured `/Users/ianburke/Documents/GitHub/PLEXI/packages/pi-voice` extension (`~/.pi/agent/settings.json`) and exposing cloud OpenAI Realtime / Whisper / LLM distillation paths (`pi-speech/src/host.ts:85-176`).
4. **Protocol & DoS nuances:** `readJson()` buffers up to `MAX_BODY = 40 MB` into Hub heap memory **before** authenticating the request (`src/hub.ts:66, 540, 569, 711-724`), and any already-paired phone has `role: "control"` and can call `rpc("pair", {})` (`src/hub.ts:648-650`, `web/app.js:1797-1826`) to mint a pairing link for a second device without terminal confirmation.

---

## Detailed Security Findings

### 1. Network Listeners & Bind Addresses

| Component | Bind / Target Address | Transport & Auth | Source Reference |
|---|---|---|---|
| **Hub HTTP server** | `127.0.0.1:<port>` (default `7878`) | Loopback-only Node `http.createServer`; never binds `0.0.0.0` or LAN | `src/hub.ts:176-189`, `src/store.ts:52` |
| **Peer Pi session link (`PeerLink`)** | `127.0.0.1:<port>` (`Upgrade: pi-mobile-link`) | Loopback remote-IP check (`127.0.0.1`, `::1`, `::ffff:127.0.0.1`) + `x-pi-mobile-secret` (`timingSafeEqual` against `~/.pi/agent/pi-mobile/link.secret`, mode `0600`) | `src/hub.ts:272-283`, `src/link.ts:168-174`, `src/store.ts:152-162` |
| **Tailscale Serve proxy** | `https://<self-dns-name>:<443\|8443\|10000>` → `http://127.0.0.1:7878` | Runs `tailscale serve --bg --https=<port> http://127.0.0.1:7878` after checking `tailscale status --json` and `tailscale serve status --json`. Exposed **only to the user's tailnet** (`*.ts.net`), never `tailscale funnel`. | `bin/tailscale.mjs:14, 106-141`, `src/hub.ts:204-227`, `src/link.ts:31` |
| **Remote SSH tunnel (optional `/remote add`)** | Remote `127.0.0.1:7878` → local `127.0.0.1:<port>` | Only active if `npx pi-mobile add <ssh-host>` is explicitly used; remote Pi connects over SSH port-forward using `enrol` token in `~/.pi/agent/pi-mobile/hub.json` (`0600`) | `bin/pi-mobile.mjs:130-152`, `src/hub.ts:261-319`, `src/link.ts:155-184` |

**Findings:**
- **No LAN / `0.0.0.0` listener:** `Hub.listen()` calls `this.server.listen(this.config.port, "127.0.0.1", ...)` (`src/hub.ts:181-189`). If port `7878` is already bound by another local Pi process, `Bridge` connects to `127.0.0.1:7878` over loopback (`src/link.ts:25-48, 86-140`); if the hub holder exits, remaining linked Pi processes re-elect a new hub on `127.0.0.1:7878` after a random 80–280 ms jitter (`src/link.ts:144-160`).
- **Automatic `tailscale serve` mutation:** Unless `config.tailscale === false` (`~/.pi/agent/pi-mobile/config.json`), `PI_MOBILE_TAILSCALE=off` (`src/link.ts:31`), or `config.publicUrl` is set (`src/hub.ts:206`), starting the hub automatically executes `tailscale serve --bg --https=<port> http://127.0.0.1:7878` (`bin/tailscale.mjs:123-141`) on the first available port in `[443, 8443, 10000]`. Neither `bin/tailscale.mjs` nor `src/hub.ts` ever runs `tailscale serve ... off`, meaning background Tailscale Serve rules persist across Pi restarts once enabled until manually removed with `tailscale serve --https=<port> off`.
- **Same-machine loopback exposure:** Any process running on `localhost` can connect to `127.0.0.1:7878` to fetch unauthenticated static web assets (`/`, `/app.js`, `/style.css`, `/nacl.js`, `/qr.js` at `src/hub.ts:471-505`), though `/api/rpc`, `/api/stream`, and `Upgrade: pi-mobile-link` require either a paired device key (`devices.json`, `0600`) or `~/.pi/agent/pi-mobile/link.secret` (`0600`).

---

### 2. Authentication, Pairing, and Cryptography Model

| Mechanism | Implementation Details | Source Reference |
|---|---|---|
| **Primitives** | `tweetnacl` (`nacl.secretbox` = XSalsa20-Poly1305 authenticated encryption, 32-byte key, 24-byte random nonce from `crypto.randomBytes` on Node / `crypto.getRandomValues` in browser). | `src/crypto.ts:1-50`, `web/app.js:35-69` |
| **Pairing token generation** | `HubServer.createPairing()` generates a 9-byte (72-bit) `id` (`randomId(9)`) and 32-byte (256-bit) `key` (`randomKey()`) with `PAIRING_TTL = 10 * 60_000` (10 minutes). | `src/hub.ts:65, 363-370` |
| **Out-of-band key delivery** | Pairing URL puts `id` and `key` in the URL hash fragment: `https://<host>/#p=<id>.<b64u(key)>`. The browser strips the hash immediately via `history.replaceState(null, "", location.pathname)` (`web/app.js:135`) so the key is never sent in HTTP URLs or Referer headers. | `src/hub.ts:368-369`, `web/app.js:132-160` |
| **Pairing exchange (`POST /api/pair`)** | Client sends `{ p: id, b: seal(pairingKey, { t: Date.now(), name }) }`. Hub verifies `open(pairing.key, body.b)` and `ReplayGuard.accept("pair:" + id, nonce, t)`, deletes the one-time pairing immediately (`this.pairings.delete(pairing.id)`), mints a persistent 32-byte per-device key (`randomKey()`), stores `{ id, name, key, role: "control", createdAt }` in `~/.pi/agent/pi-mobile/devices.json` (`0600`), and returns `{ b: seal(pairing.key, { d, k, role, host, machine }) }`. | `src/hub.ts:539-557`, `src/store.ts:69-74, 100-104` |
| **Browser key storage** | The phone browser stores `{ d, k, role, host, machine }` in `localStorage.getItem("pi-mobile.creds.v2")` (`web/app.js:91-118`). Any XSS on the same origin (`https://<machine>.<tailnet>.ts.net:<port>`) could read `localStorage`. | `web/app.js:91-118` |
| **RPC authentication (`POST /api/rpc`)** | Client sends `{ d: deviceId, b: seal(deviceKey, { t: Date.now(), op, ... }) }`. `authenticate()` looks up `deviceId` in `devices.json`, opens the NaCl secretbox, verifies `typeof payload.t === "number"`, and checks `ReplayGuard.accept(device.id, nonce, payload.t)`. Response is returned sealed: `{ b: seal(device.key, result) }`. | `src/hub.ts:559-579` |
| **Replay protection (`ReplayGuard`)** | Rejects timestamps outside `±10 minutes` (`DEFAULT_WINDOW_MS = 10 * 60_000`) and tracks `<scope>:<nonce>` in memory + persists to `~/.pi/agent/pi-mobile/seen.json` (`0600`) across restarts, pruning entries older than `20 minutes`. | `src/crypto.ts:52-95`, `src/hub.ts:167` |
| **SSE stream (`GET /api/stream`)** | Handshake authenticates query params `?d=<id>&b=<seal(deviceKey, { t, agent, cursor })>` through `authenticate()`. Subsequent `data:` frames are `seal(device.key, batch)` JSON arrays of sequenced events (`seq`). | `src/hub.ts:455-461, 664-685` |

**Security Gaps & Nuances Identified in Auth/Crypto:**
1. **Single-step QR pairing (no desktop confirmation click):** Unlike Plexi's relay pairing (`docs/security/relay-threat-model.md`, which requires a desktop confirmation click after the phone scans the QR), scanning the `/remote` QR in `pi-mobile` immediately completes pairing (`src/hub.ts:539-557`). Anyone who views the terminal QR within 10 minutes before it is consumed can pair.
2. **Paired device can mint new pairings (`rpc("pair")`):** Every paired phone gets `role: "control"` (`src/hub.ts:547`). In `dispatchRpc` (`src/hub.ts:648-650`), any `control` device can call `op: "pair"` (exposed in the mobile UI as "Pair another device" at `web/app.js:1777, 1797-1826`), minting a fresh pairing URL/QR without any prompt on the desktop terminal.
3. **40 MB pre-authentication body buffering (`MAX_BODY`):** `handlePair` (`src/hub.ts:540`) and `handleRpc` (`src/hub.ts:569`) call `readJson(req)` (`src/hub.ts:711-724`), which buffers up to `MAX_BODY = 40 * 1024 * 1024` (40 MB, sized for voice audio blobs at `src/hub.ts:66`) and runs `JSON.parse` **before** verifying `deviceId` or opening the `secretbox`. Any node on the tailnet (or any local process on `127.0.0.1:7878`) can force the Hub process to allocate 40 MB per request without a valid key.
4. **No HTTP security headers on static web shell:** `serveStatic()` (`src/hub.ts:493-505`) sets `Content-Type` and `Cache-Control: no-cache`, but does not set a `Content-Security-Policy` (`CSP`) header. Because the long-lived device key lives in `localStorage` (`web/app.js:91-118`), defense-in-depth relies entirely on the frontend escaping untrusted strings (`esc()` in `web/app.js:1836-1844`).

---

### 3. Session Data & Prompts Leaving the Host

Data leaves the Pi process across three distinct boundaries:

#### A. To the Paired Phone (over Tailscale HTTPS + `nacl.secretbox`)
- **All shared sessions' metadata (`AgentInfo`):** Agent ID, PID, session ID, session name, full `cwd`, model provider/ID, thinking level, context token usage, total cost, first line of the initial user prompt (`task`), and first line of the latest assistant message (`last`) (`src/bridge.ts:175-211`, `src/protocol.ts:8-32`).
- **Full session history & live transcript (`snapshot` / `item` / `stream`):**
  - User prompts, assistant text, and assistant `thinking` blocks (`src/bridge.ts:264-334, 511-526`, `src/protocol.ts:131-215`).
  - Tool call names, human-readable titles, clipped arguments (up to 2,000 chars in summary items; `src/protocol.ts:34, 108-129`), tool outputs (6,000 chars inline, up to **200,000 chars** via `rpc("toolOutput")` at `src/bridge.ts:395` / `src/protocol.ts:36, 231-237`), unified file diffs for `edit`/`write` (up to 12,000 chars at `src/protocol.ts:35, 319-363`), and `!bash` commands/outputs (`src/protocol.ts:165-177`).
  - Embedded base64 images from user prompts or tool results (`findImage` via `rpc("image")` at `src/bridge.ts:396-401`, `src/protocol.ts:239-270`).
- **Session list across disk (`listSessions`):** Calling `rpc("sessions")` scans `~/.pi/agent/sessions/**/*.jsonl` (`src/bridge.ts:751-824`), returning recent sessions across the machine (up to 60 sessions) and allowing the phone to switch the active Pi session to any `.jsonl` session file inside the agent's `cwd` (`src/bridge.ts:453-475`).

#### B. To Push Notification Services (only if notifications are enabled on the phone)
- **Web Push (`src/hub.ts:376-384`, `src/webpush.ts:33-93`):** If the user enables browser notifications on the phone (`rpc("pushSubscribe")` at `src/hub.ts:629-637`), the Hub encrypts `{ title: "<session> finished|asks|needs you", body: "<assistant text <=300 chars>", agent, machine }` using RFC 8291 (`aes128gcm` with the browser's P-256 `p256dh` + `auth` secret) and sends the ciphertext to the browser vendor's Web Push endpoint (Apple/Google/Mozilla). The push server sees metadata (endpoint URL, timing, payload size, VAPID public key from `~/.pi/agent/pi-mobile/push.json`) but not plaintext.
- **APNs / Push Relay (`src/hub.ts:386-400`, `src/apns.ts:74-85`, `src/relay.ts:48-73`):** Only used if a native mobile app registers an APNs token or relay capability (`rpc("apnsRegister")` / `rpc("relayRegister")`). The payload inner content is `seal(device.key, payload)`, while the outer APNs alert is static `"pi" / "New activity"` (`src/apns.ts:76`, `src/relay.ts:53`).

#### C. To Third-Party Cloud AI APIs (via bundled `pi-speech@0.1.0` when Dictation / Voice is used)
- **Config location (`pi-speech/src/config.ts:76-83, 103-109`):** `loadVoiceConfig()` reads `~/.pi/agent/pi-speech/config.json` (`speechDir()` = `join(agentDir(), "pi-speech")`), with keys defined by `VoiceConfig` (`stt?: "auto" | "apple" | "openai"`, `realtime?: "auto" | "off"`, `tts?: "auto" | "off"`, `model?: string`, `locale?: string`, etc. at `pi-speech/src/config.ts:9-36`).
- **Dictation STT (`pi-speech/src/host.ts:85-101`, `src/stt.ts:284-299, 419-444`):**
  - Extracts repository vocabulary from `cwd` (`git ls-files`, `git rev-parse --show-toplevel`, `git grep` on modified files, plus the last 30 session messages; `pi-speech/src/vocab.ts:113-140`).
  - On macOS 26+ (`darwinMajor() >= 26`), default `stt: "auto"` uses Apple's on-device `SpeechTranscriber` (`pi-speech/src/stt.ts:190-280, 419-424`). If `stt: "openai"` is configured or on older macOS when `OPENAI_API_KEY` is present in `process.env`, audio + up to 80 repo vocabulary identifiers are `POST`ed to `https://api.openai.com/v1/audio/transcriptions` (`gpt-4o-transcribe`, `pi-speech/src/stt.ts:284-299`).
- **Dictation LLM Distillation (`pi-speech/src/host.ts:86, 98-107`, `src/distill.ts:73-112`, `src/model.ts:77-112`):**
  - `distill` is a **per-request parameter** (`p.distill === false` in `VoiceHost.transcribe(p)` at `pi-speech/src/host.ts:86, 98`, passed by `/dictate raw` in the terminal), **not** a field in `~/.pi/agent/pi-speech/config.json` (`VoiceConfig`). Whenever `p.distill !== false` (the phone default) or when the phone calls `distill`, `brief`, `details`, or `draftFinish` (`src/bridge.ts:435-468`), `VoiceHost` sends the spoken transcript / run digest + repo vocabulary + recent session messages to a fast model in Pi's `ctx.modelRegistry` (`registryComplete` at `pi-speech/src/model.ts:77-112`) using the active Pi provider credentials.
- **Realtime Voice Mode (`pi-speech/src/host.ts:142-176`, `src/realtime.ts:248-337`, `src/tts.ts:22-67`):**
  - Unless `realtime: "off"` is set in `~/.pi/agent/pi-speech/config.json` (`pi-speech/src/config.ts:17`, checked at `pi-speech/src/host.ts:152`), starting voice mode on the phone (`voiceSession` at `src/bridge.ts:439, 463`) or terminal (`/voice`) reads `process.env.OPENAI_API_KEY` or `ctx.modelRegistry.getApiKeyForProvider("openai-codex")` (`pi-speech/src/host.ts:142-147`) and `POST`s the WebRTC SDP offer + system instructions (including project name and 60 repo vocabulary symbols) to `https://chatgpt.com/backend-api/codex/realtime/calls` or `https://api.openai.com/v1/realtime/calls` (`gpt-realtime-2`).
  - Unless `tts: "off"` is set in `~/.pi/agent/pi-speech/config.json` (`pi-speech/src/config.ts:29`, checked by `ttsTarget` at `pi-speech/src/tts.ts:24`), fallback TTS (`pi-speech/src/tts.ts:34-43`) sends text to `https://api.openai.com/v1/audio/speech` (`gpt-4o-mini-tts`) when `OPENAI_API_KEY` is set.

---

### 4. Remote Code Execution, Telemetry, and Local Side Effects

- **Zero telemetry or analytics:** Every outbound network sink in `pi-mobile@0.4.0` and `pi-speech@0.1.0` was audited (`fetch`, `http.request`, `https.request`, WebSocket, WebRTC). There are no telemetry, analytics, crash-reporting, or author-controlled servers anywhere in either package.
- **Phone-triggered agent execution (`rpc("send")`):** By design, a paired phone with `role: "control"` can send user prompts (`pi.sendUserMessage` at `src/bridge.ts:421-443`), abort runs (`ctx.abort()` at `src/bridge.ts:391`), switch models (`pi.setModel` at `src/bridge.ts:492-502`), change thinking levels (`pi.setThinkingLevel` at `src/bridge.ts:504-509`), and switch/fork sessions (`src/bridge.ts:445-490`). Because Pi has bash and file-editing tools enabled in interactive sessions, **anyone holding a paired phone has equivalent code-execution authority to typing a prompt into that Pi terminal**.
- **On-demand Swift binary compilation (`pi-speech/src/stt.ts:137-188`):** When dictation or `/voice` is invoked on macOS, `appleHelper()` compiles `pi-speech/src/apple-stt.swift` via `xcrun swiftc -parse-as-library -O <src> -o ~/.pi/agent/pi-speech/bin/apple-stt-<sha256>` and executes it. It does not run at extension load time if dictation/voice is untouched.
- **Explicit SSH remote enrollment (`bin/pi-mobile.mjs:130-152`):** Running `/remote add <ssh-host>` or `npx pi-mobile add <ssh-host>` runs `ssh -o BatchMode=yes <host> -- npx -y pi-mobile@0.4.0 enrol` (`bin/pi-mobile.mjs:135`). This only runs when explicitly invoked by the user.
- **Plaintext dictation drafts on disk (`pi-speech/src/drafting.ts:43-128`):** Long dictations write temporary WAV/text drafts under `~/.pi/agent/pi-speech/drafts/` (`0700` dir, `0600` files) retained for 7 days (`DRAFT_TTL_MS = 7 * 24 * 60 * 60 * 1000`).

---

### 5. Dependency & Supply-Chain Risk (`npm audit`)

#### Direct Dependencies (`package.json:52-56`)
1. `tweetnacl@^1.0.3`: Zero transitive dependencies; audited public-domain TweetNaCl JS port.
2. `qrcode@^1.5.4`: Pulls `dijkstrajs`, `pngjs`, `yargs` (standard QR renderer).
3. `pi-speech@^0.1.0`: Same upstream repo (`938d53dd...`). Pulls:
   - `opusscript@^0.1.1` (Emscripten-compiled Opus codec for terminal WebRTC audio).
   - `werift@^0.25.0` (Pure-TypeScript WebRTC implementation, dynamically imported via `await import("werift")` in `pi-speech/src/rtc.ts:68` **only** when terminal `/voice` is invoked).

#### Lifecycle Scripts & `npm audit` Results
- **Lifecycle scripts:** Neither `pi-mobile@0.4.0` nor `pi-speech@0.1.0` defines `preinstall`, `install`, or `postinstall` scripts (`package.json`, `packages/pi-speech/package.json`).
- **Production dependency tree (`npm audit --omit=dev`):** **0 vulnerabilities** across 185 production packages (207 including optional native audio helpers). (The large transitive count comes almost entirely from `werift@^0.25.0` in `pi-speech`.)
- **Dev dependency tree (`npm audit` including `devDependencies`):** 7 `high` advisories, all inside `puppeteer-core@^24.0.0` (`extract-zip`, `basic-ftp`, `@puppeteer/browsers`), which is a `devDependency` of `pi-mobile` (`package.json:60`) and is **not** installed when installing `pi-mobile` as a production package.
- **Vendored browser assets (`web/`):** `web/qr.js` (vendored `jsQR 1.4.0`, Apache-2.0, loaded only when opening the in-browser QR scanner) and `tweetnacl/nacl-fast.min.js` (served from `node_modules/tweetnacl/nacl-fast.min.js` at `src/hub.ts:518-521`).

#### Extension Collision with Plexi's `packages/pi-voice` (Stint `0839`)
- Ian's `~/.pi/agent/settings.json` already loads `/Users/ianburke/Documents/GitHub/PLEXI/packages/pi-voice` (local Parakeet v3 STT + Kitten TTS).
- `pi-mobile/src/index.ts:88-98` unconditionally calls `registerDictation(pi, complete)` and `registerVoiceMode(pi, complete)` from `pi-speech`, registering a competing `/voice` slash command (`pi-speech/src/voice.ts:289`), and `src/index.ts:122` attaches `b.voice = voice` (`VoiceHost`), which exposes the phone speech RPCs (`transcribe`, `distill`, `brief`, `details`, `voiceSession`, `speak`, `voices`, `learn`, `voiceState`, `draftStart`, `draftChunk`, `draftState`, `draftFinish`, `draftEdit`, `draftClose` in `src/bridge.ts:435-468`). Simply removing `registerDictation` / `registerVoiceMode` stops the terminal slash-command collision, whereas removing `b.voice = voice` (`src/index.ts:122`) is required to disable phone speech RPCs completely (`src/bridge.ts:451` then rejects them with `"voice is not available in this pi"`).

---

### 6. How `/remote` Exposes the Pi Coordinator (and Other Sessions)

`pi-mobile` has two sharing modes controlled by `shouldShare()` (`src/index.ts:148`):

```ts
const shouldShare = () => !g[HIDDEN] && (loadConfig().enabled || (pi.getFlag("remote") === true && !g[FLAG_OFF]));
```

1. **What `/remote` and `/remote pair` do (`src/index.ts:278-292, 319-320`):**
   - Both `connect()` (`/remote`) and `pair()` (`/remote pair`) call `enable()` (`src/index.ts:278-282`), which clears `g[HIDDEN]` and writes `"enabled": true` to `~/.pi/agent/pi-mobile/config.json` whenever `cfg.enabled` is `false`.
   - Because **every** running Pi process with `pi-mobile` loaded polls `~/.pi/agent/pi-mobile/config.json` every 1,500 ms via `watchFile(configPath(), { interval: 1500, persistent: false }, sync)` (`src/index.ts:167-171`) and checks `shouldShare()` on `session_start` (`src/index.ts:148-158`), flipping `config.enabled = true` **automatically exposes every active and future Pi session on the machine** to the hub and paired phones!
   - Ephemeral worker panes spawned by `plexi agent spawn` pass `--no-extensions` (stint `0842`), so they do not load `~/.pi/agent/` packages, **but** any interactive Pi pane (including `babysitter-v3` coordinator/worker panes or ad-hoc `pi` terminals) that loads extensions will join the hub automatically whenever `config.enabled === true`.
2. **Per-session sharing (`pi --remote` with `config.enabled: false`):**
   - Starting Pi with `pi --remote` sets `pi.getFlag("remote") === true` (`src/index.ts:85, 148`) for **that single Pi process only**, so it links to the hub on startup without needing `config.enabled === true`.
   - However, because `connect()` and `pair()` both call `enable()` (`src/index.ts:278-282, 286, 320`), running `/remote` or `/remote pair` inside a `pi --remote` session still writes `"enabled": true` to `~/.pi/agent/pi-mobile/config.json` unless `enable()` is patched not to mutate `config.json` when `pi.getFlag("remote") === true` (or `"enabled": false` is restored in `config.json` immediately after pairing).

---

### 7. Stint `0788` Click-Only Approval Analysis

Does `pi-mobile` allow a paired phone to bypass Plexi's stint `0788` click-only rule for host approvals and secret reads?

| Gate / Prompt Surface | Enforced By | Can `pi-mobile` Approve / Resolve It from Phone? | Why (Symbol & File Reference) |
|---|---|---|---|
| **Mutating app-tool approval (`NeedsYouKind::ApprovalClick`)** | Rust host broker (`PermissionMonitor::resolve_needs_you_from` + `NeedsYouKind::surface_may_resolve`) & non-synthetic egui click (`draw_approval_banner`) | **NO** | `NeedsYouKind::surface_may_resolve` (`src/broker/gate.rs:101-108`) returns `false` for `approve: true` on `ResolveSurface::Socket` and `ResolveSurface::Phone`. Any CLI/socket attempt from the Pi pane logs `refused_resolve` and returns `permission_denied`. |
| **Agent permission-widen request (`NeedsYouKind::PermissionChange`)** | Rust host broker (`NeedsYouKind::surface_may_resolve`) | **NO** | `ResolveSurface::Socket` cannot approve (`src/broker/gate.rs:101-108, 1558-1635`). Requires a real desktop click in the Plexi window. |
| **Seal / journal fault (`NeedsYouKind::Integrity`)** | Rust host broker (`NeedsYouKind::surface_may_resolve`) | **NO** | Neither `approve` nor `deny` is permitted from `ResolveSurface::Socket` or `ResolveSurface::Phone` (`src/broker/gate.rs:104-106`). |
| **Keychain signing-key recovery (`NeedsYouKind::Keychain`)** | Rust host broker (`NeedsYouKind::surface_may_resolve`) | **NO** | Neither `approve` nor `deny` is permitted from `ResolveSurface::Socket` or `ResolveSurface::Phone` (`src/broker/gate.rs:104-106, 1341`). |
| **Secret read (`plexi secret get <KEY>`)** | Rust host command handler (`src/app/secret_get.rs`, `src/workspace/secrets/pane_read.rs`) | **NO** | `plexi secret get` blocks waiting for a real pointer click on the desktop `draw_approval_banner` (`!synthetic_frame && any_released`) and cannot be approved over `PLEXI_SOCKET` or by `ctx.ui`. |
| **Creative / blocking question (`NeedsYouKind::Question` / `BlockedRun`) when `prompt_in_pi_ui: true`** | `PermissionMonitor::answer_needs_you_from` (`src/broker/gate.rs:1353-1403`) + `@plexi/pi` `needsYouAskTool` (`packages/pi-plexi/src/tools.ts:410-425`) | **YES (answers question only; never mints a grant)** | When `plexi_needs_you_ask` is called with `wait_for_answer: true` and `prompt_in_pi_ui: true`, `@plexi/pi` holds the host-minted one-time `ui_token` and calls `ctx.ui.select(...)` (`packages/pi-plexi/src/tools.ts:410-420`). Because `pi-mobile`'s `patchUI` (`src/bridge.ts:588-605`) monkey-patches `ctx.ui.select` to forward the prompt to paired phones (`rpc("answer")`), a phone answer resolves `ctx.ui.select` inside the Pi process, which then submits `ui_token` to `plexi needs-you answer`. Note: `answer_needs_you_from` strictly checks `matches!(kind, Some(NeedsYouKind::Question \| NeedsYouKind::BlockedRun))` (`src/broker/gate.rs:1369-1372`) and **never mints a `GrantRecord`**. |
| **Pi-internal extension dialogs (`ctx.ui.select` / `confirm` / `input`) & user prompts (`pi.sendUserMessage`)** | Pi extension runtime inside the terminal pane | **YES** | `patchUI` (`src/bridge.ts:588-605`) and `Bridge.send` (`src/bridge.ts:528-552`) explicitly bridge Pi TUI dialogs and user turns to the phone. |

**Bottom line on `0788`:**
`pi-mobile` **cannot** weaken or bypass `0788` for any security-granting (`ApprovalClick`, `PermissionChange`), integrity (`Integrity`), Keychain (`Keychain`), or secret-reading (`plexi secret get`) gate, because those gates are enforced in the Rust host process (`PermissionMonitor`) on real desktop pointer events and reject all `ResolveSurface::Socket` approvals. The only cross-over is non-grant `NeedsYouKind::Question` items when `prompt_in_pi_ui: true` is used, where `pi-mobile`'s `ctx.ui.select` monkey-patch allows the phone to pick a choice using `@plexi/pi`'s `ui_token`.

---

## Recommendation

**Install only with Config & Guardrails ("Install with Config X") — do NOT install unconfigured.**

Specifically:
1. **Pin exact version `0.4.0`** (never floating `npm:pi-mobile`) and verify the tarball SHA-256 (`b9c6b512201fd1c3b347524b93781497d74dab02a27ecb5728931cde8e453c97`) before loading.
2. **Pre-seed `~/.pi/agent/pi-mobile/config.json` with `"enabled": false` and `"tailscale": false`** so `pi-mobile` never auto-shares all Pi sessions on startup and never mutates `tailscale serve` behind the scenes.
3. **Neutralize the `pi-speech` `/voice` collision and cloud-speech egress:**
   - Pre-seed `~/.pi/agent/pi-speech/config.json` (`0600`, read by `loadVoiceConfig()` at `pi-speech/src/config.ts:76-83, 103-109`) with `"stt": "apple"`, `"realtime": "off"`, and `"tts": "off"` (`pi-speech/src/config.ts:11, 17, 29`) so `pi-speech` never calls OpenAI Whisper (`pi-speech/src/stt.ts:419-424`), OpenAI/ChatGPT Realtime (`pi-speech/src/host.ts:152`), or OpenAI TTS (`pi-speech/src/tts.ts:24`). Note that `distill: false` is a per-request parameter on `VoiceHost.transcribe(p)` (`pi-speech/src/host.ts:86, 98`), not a config file setting, so phone speech RPCs (`transcribe`, `distill`, `brief`, `details`, `draftFinish` at `src/bridge.ts:435-468`) will still invoke `registryComplete` (`pi-speech/src/model.ts:77-112`) if `b.voice` is attached.
   - Because Ian's `~/.pi/agent/settings.json` already loads `/Users/ianburke/Documents/GitHub/PLEXI/packages/pi-voice` (`0839`), unpack `pi-mobile-0.4.0.tgz` locally and comment out `registerDictation(pi, complete)` and `registerVoiceMode(pi, complete)` (`src/index.ts:92-93`) to prevent `/voice` and `/dictate` slash-command collisions — and also comment out `b.voice = voice` (`src/index.ts:122`) if you want phone speech RPCs completely disabled (`src/bridge.ts:451`).
4. **Opt in per-session with `pi --remote`** on the specific Pi coordinator session you want on the phone, and note that both `/remote` and `/remote pair` call `enable()` (`src/index.ts:278-286, 319-320`), which sets `"enabled": true` in `~/.pi/agent/pi-mobile/config.json` unless `enable()` is patched in the local unpacked copy or `"enabled": false` is restored after pairing.

---

## Approved Install & Tailscale Runbook

If Ian approves installing `pi-mobile` for his Pi coordinator, run these exact steps in an external terminal:

### Step 1: Pre-seed locked-down configs (`0700` dirs, `0600` files)

`pi-mobile` reads `~/.pi/agent/pi-mobile/config.json` (`src/store.ts:54-67`) and `pi-speech` reads `~/.pi/agent/pi-speech/config.json` (`pi-speech/src/config.ts:76-83, 103-109`):

```bash
mkdir -p -m 700 ~/.pi/agent/pi-mobile ~/.pi/agent/pi-speech

cat > ~/.pi/agent/pi-mobile/config.json <<'EOF'
{
  "enabled": false,
  "port": 7878,
  "tailscale": false
}
EOF
chmod 600 ~/.pi/agent/pi-mobile/config.json

cat > ~/.pi/agent/pi-speech/config.json <<'EOF'
{
  "stt": "apple",
  "realtime": "off",
  "tts": "off"
}
EOF
chmod 600 ~/.pi/agent/pi-speech/config.json
```

### Step 2: Verify tarball hash and install pinned `pi-mobile@0.4.0`

```bash
TMP_AUDIT="$(mktemp -d)"
cd "$TMP_AUDIT"
npm pack pi-mobile@0.4.0
shasum -a 256 pi-mobile-0.4.0.tgz
# MUST match:
# b9c6b512201fd1c3b347524b93781497d74dab02a27ecb5728931cde8e453c97  pi-mobile-0.4.0.tgz

pi install npm:pi-mobile@0.4.0
```

*(Note: Because `/Users/ianburke/Documents/GitHub/PLEXI/packages/pi-voice` is active in `~/.pi/agent/settings.json`, unpack `pi-mobile-0.4.0.tgz` to a local directory, comment out `registerDictation(pi, complete)` and `registerVoiceMode(pi, complete)` at `src/index.ts:92-93` to avoid the terminal `/voice` command collision, and also comment out `b.voice = voice` at `src/index.ts:122` so phone speech RPCs in `src/bridge.ts:435-468` are rejected at `src/bridge.ts:451` rather than calling `registryComplete` for distillation/briefs; then install that local directory via `pi install <path>`.)*

### Step 3: Configure Tailscale Serve explicitly (tailnet-only HTTPS, no Funnel)

Run `tailscale serve` explicitly on a dedicated high port (`8443`) so you control lifecycle and verify that public Funnel is **off**:

```bash
# 1. Verify tailnet connection
tailscale status

# 2. Expose 127.0.0.1:7878 to your tailnet ONLY on https://<machine>.<tailnet>.ts.net:8443
tailscale serve --bg --https=8443 http://127.0.0.1:7878

# 3. Verify Serve status shows tailnet-only "(tailnet only)" and NO Funnel
tailscale serve status
```

Record the `https://<machine>.<tailnet>.ts.net:8443` URL in `~/.pi/agent/pi-mobile/config.json`:

```bash
cat > ~/.pi/agent/pi-mobile/config.json <<'EOF'
{
  "enabled": false,
  "port": 7878,
  "tailscale": false,
  "publicUrl": "https://<machine>.<tailnet>.ts.net:8443"
}
EOF
chmod 600 ~/.pi/agent/pi-mobile/config.json
```

### Step 4: Start only the intended Pi coordinator session with `--remote` and pair

```bash
# Launch only the Pi coordinator session with remote sharing enabled:
pi --remote
```

Inside that Pi coordinator session:
1. Run `/remote pair` and scan the one-time QR code with your phone on the same tailnet within 10 minutes.
2. Because `pair()` calls `enable()` (`src/index.ts:278-282, 319-320`), which writes `"enabled": true` to `~/.pi/agent/pi-mobile/config.json`, immediately reset `"enabled": false` in `~/.pi/agent/pi-mobile/config.json` after pairing (or no-op `enable()` in `src/index.ts:278-282` when `pi.getFlag("remote") === true`) so other Pi sessions on the machine do not auto-join while the `pi --remote` coordinator session stays shared via its `--remote` flag (`src/index.ts:148`).
3. Verify paired devices with `/remote devices`.

### Step 5: Teardown / Revocation

`bin/pi-mobile.mjs` (`bin/pi-mobile.mjs:221-263`) only supports multi-machine SSH commands (`add <ssh-host>`, `remove <name>`, `machines`, `enrol`); device revocation and stopping sharing are interactive `/remote` subcommands inside Pi (`src/index.ts:227-230, 370-401`):

1. **Inside Pi (interactive):**
   - Run `/remote devices` → select a device or **`Revoke all`** → confirm (`src/index.ts:387-401`, which calls `hubRequest("revoke", { ids })` at `src/hub.ts:641-647` to delete keys from `~/.pi/agent/pi-mobile/devices.json`, clear push subscriptions, and drop active SSE streams).
   - Run `/remote off` → select **`Only this session`** or **`Every session on this machine`** (`src/index.ts:370-385`).
2. **From an external shell (when Pi is not running, plus Tailscale Serve teardown):**
   ```bash
   # Disable machine-wide sharing and wipe paired device keys on disk:
   cat > ~/.pi/agent/pi-mobile/config.json <<'EOF'
   {
     "enabled": false,
     "port": 7878,
     "tailscale": false
   }
   EOF
   printf '[]\n' > ~/.pi/agent/pi-mobile/devices.json
   chmod 600 ~/.pi/agent/pi-mobile/config.json ~/.pi/agent/pi-mobile/devices.json

   # Remove the Tailscale Serve rule:
   tailscale serve --https=8443 off
   tailscale serve status
   ```
