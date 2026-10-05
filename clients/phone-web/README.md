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

Host mode runs `plexi assistant send --json` locally and requires a bearer token even on loopback. The server prints a ready URL containing that token; the page stores it for the session and removes it from the address bar. Each send is answered by the `turn_id` that command created. A desktop permission prompt returns `waiting_for_permission` immediately, with the pending request id, instead of waiting out the host timeout. Approval stays on the desktop.

With `--lan`, open the first URL labelled as the default-route LAN address on the phone. Bridge, VPN, and other virtual-interface addresses are skipped (except a Tailscale address when available).

Not claimed: HTTPS, a relay, third-party logins, persistence, install prompts over LAN, or protection from observers on a plain HTTP LAN (the token is visible on that network).
