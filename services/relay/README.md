# Phone relay

Small in-memory relay between a paired phone and one desktop host.

The desktop connects outbound (`plexi relay connect --url ws://127.0.0.1:8787` locally, `wss://` in production). The phone uses HTTPS (plain HTTP is for local runs). Message bodies live only in process memory: they are dropped once delivered, or after 120 seconds if the desktop is offline. Logs carry ids and byte counts, never bodies.

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

Confirm the pairing the phone redeems, then revoke that device from the desktop. Both talk to the running `relay connect` process; the URL is not repeated:

```sh
plexi relay confirm
plexi relay revoke <device-id>
```

Phone turns run as `assistant send --text … --conversation-id phone-<host>`. The phone cannot approve irreversible actions; a pending approval returns "waiting on desktop".

## Checks

```sh
python3 -m unittest services/relay/test_relay.py
bash services/relay/e2e_local.sh
```

The container image is `services/relay/Dockerfile`. Staging notes that are not executed live in `DEPLOY.md`.
