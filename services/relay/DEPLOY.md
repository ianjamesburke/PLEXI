# Railway staging

Staging is https://plexi-relay-staging.up.railway.app. That host was deployed from outside this tree. This file does not deploy, and it does not touch Cloudflare or Fly.

`railway.toml` at the repository root is what `railway up` from that root reads:

- `build.builder` is `DOCKERFILE`.
- `build.dockerfilePath` is `services/relay/Dockerfile`.
- Leave the service root directory at `/`, the repository. Railway uses that directory as the Docker build context, so the Dockerfile copies `services/relay/relay.py` and `clients/phone-web/static` in place. Do not set the root directory to `services/relay`, and do not flatten those files into a second context.
- The image command is the Dockerfile `CMD`. The process listens on `$PORT`.
- TLS: Railway terminates HTTPS and `wss://` at the edge. The process itself listens on plain HTTP and WebSocket. The desktop uses `plexi relay connect --url wss://plexi-relay-staging.up.railway.app`.
- Storage: do not attach a volume. Pairings and undelivered bodies stay in memory.
- Health: `GET /healthz` returns `{"ok":true}`. `deploy.healthcheckPath` in `railway.toml` is `/healthz`.

The website service keeps its own config at `website/railway.json`. Do not run `railway up` from this change.
