# Phone shell

Phone-sized web page for Plexi: connection state, one transcript, a composer, Send and Cancel. It is the phone shell waiting for the real intake contract (spec P4). It is **not** the cloud assistant: there is no auth, no pairing, no host connection, and no persistence. `server.py` accepts turns into memory, marks them `queued`, and echoes them back after a short delay so cancellation can be tested.

Stack: Python stdlib server + static HTML/CSS/JS on one origin. No dependencies.

```sh
python3 clients/phone-web/server.py --port 8787 # loopback stub (default)
PLEXI_BIN=plexi-pr-2680 python3 clients/phone-web/server.py --backend host --port 8788
PLEXI_BIN=plexi-pr-2680 python3 clients/phone-web/server.py --backend host --lan --port 8788
```

Checks:

```sh
python3 -m unittest clients/phone-web/test_server.py
node --experimental-websocket clients/phone-web/browser_check.mjs http://127.0.0.1:8787   # needs google-chrome and a running server
```

`browser_check.mjs` runs headless Chrome in a 390x844 mobile viewport. That is browser emulation, not a physical phone check.

Host mode runs `plexi assistant send` locally and requires a bearer token even on loopback. The server prints a ready URL containing that token; the page stores it for the session and removes it from the address bar.

With `--lan`, open the first URL labelled as the default-route LAN address on the phone. Bridge, VPN, and other virtual-interface addresses are skipped (except a Tailscale address when available).

The default phone path is the relay (`services/relay/`), which serves this same page. `--tailscale` is an optional direct alternative: it binds to the address from `tailscale ip -4` and prints that URL (plus MagicDNS when `tailscale status --json` provides one).

Not claimed: HTTPS on the local stub, third-party logins, persistence, install prompts over LAN, or protection from observers on a plain HTTP LAN (the token is visible on that network).
