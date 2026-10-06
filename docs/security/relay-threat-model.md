# Phone relay threat model

Review of the phone relay on `cursor/phone-relay-c1-fc2b` (PR 2685), plus the local fixes in this branch. Scope is `services/relay/`, the desktop client in `src/cli/relay.rs` and `src/cli/relay_ws.rs`, and the phone static under `clients/phone-web/` (including the local shell in `clients/phone-web/server.py` that serves it).

This is a review, not a PRM. It does not deploy anything. Staging stays `https://plexi-relay-staging.up.railway.app`. The relay process listens on plain HTTP; Railway terminates TLS.

## Assets

| Asset | Where it lives | Notes |
|---|---|---|
| Host token | Desktop keychain (`load_identity` in `src/cli/relay.rs`). Relay stores SHA-256 only (`hosts.token_hash`). | First hello for a `host_id` binds the hash. A later hello must match. |
| Phone session cookie | Browser cookie `plexi_phone`. Relay stores SHA-256 in memory and, when configured, in SQLite. | Raw token is not written to the registry. |
| Pairing code | Desktop screen and `relay-status.json` (mode `0o600`) for the pairing window. Relay stores SHA-256 in memory only. | 8 characters from a 32-symbol alphabet: 40 bits. TTL 300 seconds. |
| Message bodies | Relay process memory until ack or the 120 second TTL. | Not written to SQLite. Not written to the log allowlist. |
| Desktop authority | The host assistant permission gate. | The relay cannot approve a tool call. |

## Trust boundaries

```
phone browser  --HTTPS cookie-->  relay (Railway TLS, process is HTTP)
                                      |
                                      | outbound WebSocket (wss in production)
                                      v
                                 desktop host
                                      |
                                      | submit_assistant_turn
                                      v
                                 permission gate (desktop user)
```

- The phone is untrusted input. It can send text. It cannot name another host, choose the conversation id (`submit` sets `phone-{host_id}`), or approve a tool.
- The relay is a router with plaintext message bodies. It is not an authorizer.
- The desktop host is the authority boundary. A frame that arrives over the relay is still an assistant turn.
- The permission gate is the desktop user's decision. `waiting_for_permission` stays on the desktop.

## Attacker models

### Stranger on the internet

**Goal.** Pair a phone to someone else's desktop, read their traffic, or exhaust the process.

**Mitigations.**

- Pairing needs the code and a desktop confirm. Redeem of an unknown code is 404 (`redeem`, `services/relay/relay.py:722`). Confirm checks `pairing.host_id` (`confirm`, `relay.py:540`).
- Code entropy is `CODE_ALPHABET` × `CODE_LENGTH` (`relay.py:44`, `relay.py:55`): 32^8 = 40 bits, TTL `PAIRING_TTL_SECONDS` (`relay.py:46`).
- Failed guesses are capped at `PAIR_FAIL_LIMIT` (`relay.py:58`) per client address and `PAIR_FAIL_GLOBAL_LIMIT` (`relay.py:59`) per window (`redeem`, `_pair_blocked_locked` at `relay.py:830`). A limited client is not told whether the code was valid.
- Desktop sockets require hello. Unauthenticated sockets are capped (`MAX_DESKTOP_SOCKETS`, `relay.py:53`) and closed after `HELLO_DEADLINE_SECONDS` (`relay.py:54`, `serve_desktop_socket` at `relay.py:1419`).
- HTTP bodies are capped (`MAX_BODY_BYTES` `relay.py:50`, `_read_body` `relay.py:1172`). A negative `Content-Length` is rejected. It used to call `read(-1)`, which reads until EOF.
- WebSocket frames larger than the body cap are dropped before the payload is read (`ws_recv`, `relay.py:1367`).
- Phone text is capped (`MAX_TEXT_CHARS`, `relay.py:51`). In-flight bodies per device are capped (`MAX_INFLIGHT_PER_DEVICE`, `relay.py:52`).
- POST from a cross-site browser is rejected (`_post_allowed`, `relay.py:1148`: `Sec-Fetch-Site: cross-site`, and `Origin` must be `http(s)` and match `Host`).
- The session cookie is `HttpOnly`, `SameSite=Lax`, and `Secure` on a non-loopback bind or an `https` public origin (`cookie_header` `relay.py:200`, `cookies_should_be_secure` `relay.py:207`).

**Gap.** The HTTP server still has one thread per connection and no slow-read timeout. A stranger can hold many HTTP threads. Desktop sockets are capped; plain HTTP is not.

**Severity.** High for the negative `Content-Length` read and the missing Secure cookie on the container bind (`0.0.0.0` behind Railway). Both are fixed here. Remaining thread exhaustion is Medium, follow-up.

### Stolen phone cookie

**Goal.** Keep using a paired phone after the owner loses the device.

**Mitigations.**

- `revoke` (`relay.py:590`) marks the device revoked, deletes the session hash, clears the raw re-issue token, and persists that. `device_for_token` (`relay.py:802`) then returns nothing. `pairing_status` (`relay.py:764`) returns 401.
- Tests: `test_pairing_confirm_and_revoke`, `test_revoke_drops_the_live_session_and_approval_changes_nothing`, `test_secure_cookie_and_one_shot_session`.

**Gap.** A `deliver` frame already queued on the desktop socket is not cancelled. A stolen cookie can still submit new turns until someone runs `plexi relay revoke`. There is no step-up and no per-device request rate beyond the in-flight cap.

**Severity.** Medium. Revocation itself works. Queued-frame cancel and a turn rate are follow-ups.

### Malicious relay operator

**Goal.** Read phone and assistant text, forge turns, or skip the desktop gate.

**Mitigations.**

- The pair page says bodies are plaintext on the server and TLS is not end-to-end encryption.
- The operator still cannot approve. `approve` returns 403 (`relay.py:1023`). The desktop sends `submit_assistant_turn` with no grant field (`assistant_send_payload`, `src/cli/app.rs:1945`). `submit_external_turn` (`src/assistant/mod.rs:4026`) returns `waiting_for_permission` when a sheet is already up (`mod.rs:4055`), and tool calls still hit the ask gate.
- `PLEXI_RELAY_ASSISTANT=echo` skips the host. It is honored only for a loopback URL (`dispatch_from_env`, `src/cli/relay.rs:667`). The host-owned connection always uses `Dispatch::Host` (`start_host_relay`, `relay.rs:170`).

**Gap.** The operator sees every body and can forge `text` and `join_desktop` on the desktop socket. There is no signature from the phone that the desktop checks.

**Severity.** High, follow-up. Fixing it means a pairing key the relay never sees, and signed envelopes. That is not a local patch.

### Compromised relay host

**Goal.** Read memory, the SQLite file, and in-flight bodies.

**Mitigations.**

- SQLite stores host id, device id, token hashes, fingerprint, timestamps, and revoked. It does not store bodies, pairing codes, or raw tokens (schema above `PairingRegistry`, and `test_sqlite_registry_is_private_and_stores_no_secrets`).
- The file is created mode `0o600` before SQLite opens it (`_prepare_private_file`, `relay.py:162`). A group or world bit fails startup. A directory this process creates is `0o700`. Existing parents such as `/tmp` are not chmod'd. The desktop status file is also `0o600` (`write_private`, `src/cli/relay.rs:1268`).
- Bodies live only until ack or `UNDELIVERED_TTL_SECONDS` (`relay.py:45`).
- Logs go through `trace` (`relay.py:86`: allowlisted ids and sizes, no whitespace, max 80 characters) and `_BodyFilter` (drops long lines, `plexi_phone=`, `host_token`, `Bearer `, and any registered token of at least 20 characters via `note_secret` at `relay.py:111`). The phone shell drops the query string before logging (`log_message`, `clients/phone-web/server.py:489`) and hashes both bearer lengths before compare (`server.py:507`).

**Gap.** A host that can read process memory during the TTL window gets the body. Token hashes are unsalted SHA-256. That is acceptable only while tokens stay high entropy (`token_urlsafe(32)`, desktop UUID). A memory dump also gets the raw session token until the phone's first authenticated call clears `pairing.session_token`.

**Severity.** High for plaintext bodies (same follow-up as the operator: encrypt). Medium for the file mode, fixed here. Low for hash strength at the current token size.

### Brute-forcing pairing codes

**Goal.** Redeem a live code before the desktop user notices.

**Mitigations.** 40 bits, 300 second TTL, 10 failures per client address and 100 failures globally per window. Over the limit the response is 429 and does not say whether the code matched. Tests: `test_pairing_code_entropy`, `test_failed_pairing_attempts_are_rate_limited`.

**Gap.** The per-address bucket uses the TCP peer. Behind Railway every client is the proxy, so the global bucket is the one that matters. The code does not trust `X-Forwarded-For` (a client can spoof it). 100 failures per five minutes against 2^40 is not a practical online search. A botnet that rotates real source addresses is in the same global bucket.

**Severity.** Was High as an unbounded online oracle. Fixed here. Residual proxy-IP collapse is accepted because the global cap remains.

### Replay

**Goal.** Submit the same turn twice, or reuse a confirmed pairing id as a bearer.

**Mitigations.**

- A repeated `(device_id, request_id)` with the same body returns the existing delivery. A different body is 409 (`submit`).
- `pairing_id` is logged and is not a credential after the phone uses the cookie. The first confirmed poll mints one token. Later polls repeat that same token until `device_for_token` runs, then they return status only (`pairing_status`). Revoke clears the copy.
- Test: `test_secure_cookie_and_one_shot_session`.

**Gap.** Until the phone's first authenticated request, anyone who has `pairing_id` can receive that same cookie. `pairing_id` is 64 bits (`pair-` + 8 random bytes) and is in the relay log and the desktop status file.

**Severity.** Was High: a logged `pairing_id` minted a fresh session for the life of the process and rotated the real phone off. Fixed to one token, cleared on first use. The short race before first use is Low.

### Cross-host routing confusion

**Goal.** Have host A receive host B's phone traffic, or have host B's reply land on host A's phone.

**Mitigations.**

- Hello binds `host_id` to the token hash (`hello` `relay.py:481`, `secret_equal` `relay.py:188`). Desktop messages are handled with that connection's `host_id`, not a field in the frame (`handle_desktop_message`, `relay.py:1532`).
- `confirm`, `deny`, `revoke`, `ack`, and `reply` no-op or error when the record's `host_id` does not match.
- `submit` (`relay.py:919`) pushes to `self.links.get(device.host_id)` only.
- The phone's `conversation_id` is ignored. The relay sets `phone-{host_id}`.
- Test: `test_one_host_cannot_see_another_hosts_turn`.

**Gap.** None in the router. A malicious relay (the operator model) can still put any text on the socket the desktop trusts.

**Severity.** Routing confusion is not present. The operator forge is the High follow-up above.

## Permission gate

Anything that arrives via the relay is still an assistant turn.

- The phone approval routes return 403 `waiting_on_desktop` (`approve`). They do not change delivery state. Test: `test_revoke_drops_the_live_session_and_approval_changes_nothing` and `test_approval_is_waiting_on_desktop`.
- The desktop path calls `host_assistant_turn` (`src/cli/relay.rs:748`) → `assistant_send_result` → `assistant_send_payload` (`src/cli/app.rs:1945`). The JSON has no `approved`, `grant`, or `permission` field. Test: `echo_dispatch_is_loopback_only_and_the_payload_grants_nothing`.
- Production URLs must be `wss://`. `ws://` is accepted only for localhost (`parse_relay_url` `src/cli/relay_ws.rs:24`, `is_loopback` `relay_ws.rs:87`). TLS uses the webpki root set (`tls_wrap`, `relay_ws.rs:297`). The host token stays in the keychain (`load_identity`, `src/cli/relay.rs:1013`). The desktop control socket is loopback and unauthenticated (`TcpListener::bind("127.0.0.1:0")`, `relay.rs:227`).
- If a permission sheet is already pending, `submit_external_turn` answers `waiting_for_permission` and does not submit a new prompt.
- The ask gate that shows the sheet lives in the assistant (`permission_requested` in `src/assistant/model.rs`). This review does not move it.

The relay does not widen the grant-binding gap described in `docs/assistant-authority-model.md` (a grant still does not bind to arguments). Phone text is another untrusted input into that same gate. Closing the grant gap is that document's work, not a relay patch.

## What this branch changes

| Finding | Severity | Disposition |
|---|---|---|
| Unlimited pairing guesses | High | Fixed. Per-client and global failure budgets. |
| Negative `Content-Length` reads until EOF | High | Fixed. Rejected before `read`. |
| Session cookie without `Secure` on the public bind | High | Fixed. `Secure` unless loopback HTTP or `RELAY_COOKIE_SECURE=0`. |
| Confirmed `pairing_id` mints a new session forever | High | Fixed. One token, cleared once the phone presents it. |
| `PLEXI_RELAY_ASSISTANT=echo` against a remote relay skips the gate | High | Fixed. Echo is loopback-only. Host-owned sessions ignore it. |
| `ws://` to a non-loopback host sends the host token in cleartext | High | Fixed. `parse_relay_url` requires `wss://` off localhost. Desktop TLS still uses the webpki root set (`tls_wrap`). |
| Token compare can return early on a length mismatch | Medium | Fixed. `secret_equal` and the phone-shell bearer check hash both sides, then `compare_digest`. Host hashes were already fixed length. |
| SQLite file created world-readable until chmod | Medium | Fixed. Created `0o600` before open; bad mode fails startup. |
| Credential or `?token=` in a log line | Medium | Fixed. Relay filter plus the phone shell dropping the query string. |
| Cross-site browser POST | Low | Fixed. `Sec-Fetch-Site: cross-site` rejected. `SameSite=Lax` was already set. |
| Unauthenticated desktop sockets held forever | Medium | Fixed. Cap of 64 and a 10 second hello deadline. |
| In-flight body fan-out from one device | Medium | Fixed. Cap per device. Reply text capped at `MAX_TEXT_CHARS`. |
| Cross-host delivery | — | Already isolated. Test added. |
| Revoke cuts the live cookie | — | Already true. Test covers status, conversation, and re-issue. |
| Relay approves a tool | — | Already refused. Test covers the 403 and the desktop payload. |

## Follow-ups

These are larger than a local patch. Do them before the relay is the paid entry point, and before any containerized agent shares that trust story.

1. **Phone-to-desktop authenticity.** A pairing secret the relay never sees, and signed turn envelopes, so a malicious operator cannot forge `text` or `join_desktop`. Bodies stay unreadable on the relay.
2. **HTTP worker bound.** A connection cap and a header/body read deadline. Desktop sockets are already capped.
3. **Authenticate the desktop control socket.** `127.0.0.1` with no credential (`TcpListener::bind` in `run_session`). Safe only while it stays loopback. Any local user who can connect can confirm a pending pair.
4. **Cancel queued deliveries on revoke.** The cookie dies immediately; a frame already on the socket can still run.
5. **Prune pairing and delivery records.** Bodies expire; the records do not. An authenticated desktop can grow memory.
6. **Container hardening.** The image runs as root. Non-root, read-only rootfs, and dropped capabilities belong with the hosting guardrails, not a behavior change in this review.
7. **Assistant grant binding.** Owned by `docs/assistant-authority-model.md`. Phone text is in scope as untrusted input once that work starts.
8. **Status file and stdout.** `relay-status.json` and `println!("Code: {code}")` show the code to the person at the desktop. The file is mode `0o600`. `relay connect` attach prints the status JSON, which still contains the code. Do not ship that JSON to a log drain.
