# Phone shell (local stub)

Phone-sized web page for Plexi: connection state, one transcript, a composer, Send and Cancel. It is the phone shell waiting for the real intake contract (spec P4). It is **not** the cloud assistant: there is no auth, no pairing, no host connection, and no persistence. `server.py` accepts turns into memory, marks them `queued`, and echoes them back after a short delay so cancellation can be tested.

Stack: Python stdlib server + static HTML/CSS/JS on one origin. No dependencies.

```sh
python3 clients/phone-web/server.py --port 8787            # loopback only
python3 clients/phone-web/server.py --host 0.0.0.0 --port 8787  # reachable from the LAN
```

Checks:

```sh
python3 -m unittest clients/phone-web/test_server.py
node --experimental-websocket clients/phone-web/browser_check.mjs http://127.0.0.1:8787   # needs google-chrome and a running server
```

`browser_check.mjs` runs headless Chrome in a 390x844 mobile viewport. That is browser emulation, not a physical phone check.

Over plain LAN HTTP, browsers will not offer install; installing needs HTTPS (or localhost).
