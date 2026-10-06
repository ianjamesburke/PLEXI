# Railway staging

Staging is https://plexi-relay-staging.up.railway.app. That host was deployed from outside this tree. This file does not deploy, and it does not touch Cloudflare or Fly.

`railway.toml` at the repository root is what `railway up` from that root reads:

- `build.builder` is `DOCKERFILE`.
- `build.dockerfilePath` is `services/relay/Dockerfile`.
- Leave the service root directory at `/`, the repository. Railway uses that directory as the Docker build context, so the Dockerfile copies `services/relay/relay.py` and `clients/phone-web/static` in place. Do not set the root directory to `services/relay`, and do not flatten those files into a second context.
- The image command is the Dockerfile `CMD`. The process listens on `$PORT`.
- TLS: Railway terminates HTTPS and `wss://` at the edge. The process itself listens on plain HTTP and WebSocket. The desktop uses `plexi relay connect --url wss://plexi-relay-staging.up.railway.app`.
- Storage: undelivered message bodies stay in memory and are never written to disk. To keep pairings across a redeploy, mount a volume (for example `/data`) and set `RELAY_STATE_PATH=/data/relay.sqlite`. The file stores `host_id`, `device_id`, a hash of the device token, `fingerprint`, `created`, `last_seen`, `revoked`, and a hash of the host token. It does not store message bodies, pairing codes, or raw tokens. Leave `RELAY_STATE_PATH` unset and the registry stays in memory; a set path that cannot be opened stops the process. A paired device with no phone use for 30 days is deleted. Set the variable on the service when you redeploy. This change does not deploy.
- Health: `GET /healthz` returns `{"ok":true}`. `deploy.healthcheckPath` in `railway.toml` is `/healthz`.

The website service keeps its own config at `website/railway.json`. Do not run `railway up` from this change.
