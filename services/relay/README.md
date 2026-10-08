# Phone relay

Small relay between a paired phone and one desktop host.

The desktop connects outbound (`plexi relay connect --url ws://127.0.0.1:8787` locally, `wss://` in production). The desktop accepts `ws://` only for a loopback host. The phone uses HTTPS (plain HTTP is for local runs). A non-loopback bind sets the `Secure` cookie flag; `RELAY_COOKIE_SECURE=0` turns it off for an HTTP lab. Message bodies are sealed. The relay only routes the opaque envelope, in process memory, until the desktop acknowledges it or 120 seconds pass. The desktop public key is in the QR URL fragment (`#k=`), which this process never receives. Logs carry ids and byte counts, never bodies, pairing codes, or session tokens. Failed pairing guesses are rate-limited. Hello carries protocol version 1; a mismatch is an explicit error on the desktop and on the phone page. Pairing records survive a process restart when `RELAY_STATE_PATH` is set (see `DEPLOY.md`). A device unused for 30 days expires.

When the desktop socket is down, a paired phone's turn POST gets `409` with `{"error":"desktop_offline","message":"desktop offline"}`. `GET /api/status` reports the same.

## Run locally

From the repository root, so the relay can serve `clients/phone-web/static`:

```sh
python3 services/relay/relay.py --host 127.0.0.1 --port 8787
```

On the desktop, with a running Plexi host:

```sh
plexi relay connect --url ws://127.0.0.1:8787
```

The person at the desktop confirms the fingerprint with Allow once on the permission sheet. `plexi relay confirm` from an agent pane is refused. Revoke still talks to the running desktop session:

```sh
plexi relay revoke <device-id>
```

Phone turns run as `assistant send --text … --conversation <id>`. Approving a permission from the phone or the terminal is `permission_denied`. Only a desktop Allow once click grants. A phone approval of an irreversible item returns "waiting on desktop".

## Checks

```sh
python3 -m unittest services/relay/test_relay.py
bash services/relay/e2e_installed.sh
```

`e2e_installed.sh` confirms pairing with `HUMAN_APPROVE` (a real click). It does not call `relay confirm`.

The container image is `services/relay/Dockerfile`. Staging notes live in `DEPLOY.md`.
