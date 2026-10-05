# Railway staging note (not executed)

This file is a note for a later operator. Nothing in this change deploys, and it does not touch Railway, Cloudflare, or Fly.

Target shape when someone does deploy staging:

- Platform: Railway.
- Public hostname: a `*.up.railway.app` service URL.
- Start command: `python3 -u services/relay/relay.py --host 0.0.0.0 --port $PORT` from a checkout that still contains `clients/phone-web/static`, or the `services/relay/Dockerfile` (build context is the repository root).
- TLS: Railway terminates HTTPS and `wss://` at the edge. The process itself listens on plain HTTP/WebSocket inside the platform network. The desktop uses `plexi relay connect --url wss://<service>.up.railway.app`.
- Storage: do not attach a volume. The relay keeps pairings and undelivered bodies in memory only.
- Health: `GET /health` returns `{"ok":true}`.

Do not run `railway up` from this branch.
