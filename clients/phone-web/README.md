# Phone shell

Phone-sized web page for Plexi: connection state, one transcript, a composer, Send and Cancel. It is the phone shell waiting for the real intake contract (spec P4). It is **not** the cloud assistant: there is no auth, no pairing, no host connection, and no persistence. `server.py` accepts turns into memory, marks them `queued`, and echoes them back after a short delay so cancellation can be tested.

Stack: Python stdlib server + static HTML/CSS/JS on one origin. No dependencies.

```sh
python3 clients/phone-web/server.py --port 8787 # loopback stub (default)
PLEXI_BIN=plexi-pr-2680 python3 clients/phone-web/server.py --backend host --port 8788
PLEXI_BIN=plexi-pr-2680 python3 clients/phone-web/server.py --backend host --lan --port 8788
PLEXI_BIN=plexi-pr-2680 python3 clients/phone-web/server.py --backend host --tailscale --port 8788
```

Checks:

```sh
python3 -m unittest clients/phone-web/test_server.py
node --experimental-websocket clients/phone-web/browser_check.mjs http://127.0.0.1:8787   # needs google-chrome and a running server
```

`browser_check.mjs` runs headless Chrome in a 390x844 mobile viewport. That is browser emulation, not a physical phone check.

Host mode runs `plexi assistant send --json` locally and requires a bearer token even on loopback. The server prints a ready URL containing that token; the page stores it for the session and removes it from the address bar. Each send is answered by the `turn_id` that command created. A desktop permission prompt returns `waiting_for_permission` immediately, with the pending request id, instead of waiting out the host timeout. Approval stays on the desktop.

With `--lan`, open the first URL labelled as the default-route LAN address on the phone. Bridge, VPN, and other virtual-interface addresses are skipped (except a Tailscale address when available). `--lan` listens on every interface. It is the same-Wi-Fi path, not the cellular path.

## Cellular via Tailscale

Install the Tailscale app on this computer and on the phone, and sign both into the same tailnet. On the phone, open the Tailscale app and connect it. The phone can be on cellular; it does not need the computer's Wi-Fi.

```sh
PLEXI_BIN=plexi-pr-2680 python3 clients/phone-web/server.py --backend host --tailscale --port 8788
```

`--tailscale` asks `tailscale ip -4` for this machine's Tailscale IPv4 and binds only to that address. The bearer token gate stays on. The log prints that address as an `http://100.…:port/?token=…` URL. When `tailscale status --json` reports a MagicDNS name, it prints that hostname with the same token as well.

On the phone, leave the Tailscale app connected, then open the printed URL in the browser (the MagicDNS one when it is shown). If Tailscale is not running on the computer, the server exits and says so. It does not fall back to the LAN address. Do not combine `--tailscale` with `--lan`.

Not claimed: HTTPS, a relay, third-party logins, persistence, install prompts over LAN, or protection from observers on a plain HTTP LAN (the token is visible on that network). `--tailscale` is still plain HTTP; Tailscale carries it on the tailnet, and the token is still in the URL.
