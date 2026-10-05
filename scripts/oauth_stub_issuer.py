#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Local OAuth 2.0 stub issuer for `plexi connector login stub`.

A test double, never a real identity provider: it approves or denies every
authorization request without a login page, issues random opaque tokens, and
holds everything in memory. It binds loopback only.

    uv run scripts/oauth_stub_issuer.py --port 8765 [--deny] [--log /tmp/oauth-stub.log]

Endpoints:
  GET  /authorize   302 to redirect_uri with ?code= (or ?error=access_denied).
                    `stub_decision=allow|deny` on the query overrides --decision.
  POST /token       authorization_code grant; verifies the PKCE S256 verifier,
                    redirect_uri and client_id; codes are single use.
  POST /revoke      RFC 7009: revokes the token (and its pair); always 200.
  GET  /stub/state  token counts {"issued","active","revoked"} — never tokens.
"""

import argparse
import base64
import hashlib
import json
import secrets
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlencode, urlparse

CODES: dict[str, dict] = {}
# token -> {"pair": other token, "active": bool}
TOKENS: dict[str, dict] = {}
ISSUED = 0


LOG_PATH: str | None = None


def log(msg: str) -> None:
    line = f"[stub-issuer] {msg}"
    print(line, file=sys.stderr, flush=True)
    if LOG_PATH:
        with open(LOG_PATH, "a", encoding="utf-8") as output:
            print(line, file=output, flush=True)


def is_loopback_redirect(uri: str) -> bool:
    u = urlparse(uri)
    return u.scheme == "http" and u.hostname in ("127.0.0.1", "localhost", "::1")


class Handler(BaseHTTPRequestHandler):
    decision = "allow"

    def log_message(self, fmt, *args):  # route http.server noise through log()
        log(fmt % args)

    def reply(self, status: int, body: dict) -> None:
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def form(self) -> dict[str, str]:
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length).decode()
        return {k: v[0] for k, v in parse_qs(raw).items()}

    def do_GET(self):
        url = urlparse(self.path)
        q = {k: v[0] for k, v in parse_qs(url.query).items()}
        if url.path == "/stub/state":
            active = sum(1 for t in TOKENS.values() if t["active"])
            return self.reply(200, {"issued": ISSUED, "active": active, "revoked": len(TOKENS) - active})
        if url.path != "/authorize":
            return self.reply(404, {"error": "not_found"})
        redirect = q.get("redirect_uri", "")
        if not is_loopback_redirect(redirect):
            return self.reply(400, {"error": "invalid_request", "error_description": "redirect_uri must be loopback http"})
        if q.get("response_type") != "code" or q.get("code_challenge_method") != "S256" or not q.get("code_challenge"):
            return self.reply(400, {"error": "invalid_request", "error_description": "code + S256 PKCE required"})
        decision = q.get("stub_decision", self.decision)
        state = q.get("state", "")
        if decision == "deny":
            log(f"authorize: DENY client_id={q.get('client_id')}")
            params = {"error": "access_denied", "error_description": "stub issuer denied the request", "state": state}
        else:
            code = secrets.token_urlsafe(24)
            CODES[code] = {
                "challenge": q["code_challenge"],
                "redirect_uri": redirect,
                "client_id": q.get("client_id"),
                "scope": q.get("scope", ""),
            }
            log(f"authorize: ALLOW client_id={q.get('client_id')} scope={q.get('scope')}")
            params = {"code": code, "state": state}
        self.send_response(302)
        self.send_header("Location", f"{redirect}?{urlencode(params)}")
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_POST(self):
        global ISSUED
        path = urlparse(self.path).path
        f = self.form()
        if path == "/token":
            grant = CODES.pop(f.get("code", ""), None)
            if f.get("grant_type") != "authorization_code" or grant is None:
                log("token: invalid_grant (unknown or reused code)")
                return self.reply(400, {"error": "invalid_grant"})
            verifier = f.get("code_verifier", "")
            challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
            if challenge != grant["challenge"] or f.get("redirect_uri") != grant["redirect_uri"] or f.get("client_id") != grant["client_id"]:
                log("token: invalid_grant (PKCE / redirect_uri / client_id mismatch)")
                return self.reply(400, {"error": "invalid_grant"})
            access, refresh = secrets.token_urlsafe(32), secrets.token_urlsafe(32)
            TOKENS[access] = {"pair": refresh, "active": True}
            TOKENS[refresh] = {"pair": access, "active": True}
            ISSUED += 1
            log("token: issued access+refresh pair")
            return self.reply(200, {"access_token": access, "token_type": "Bearer", "expires_in": 3600,
                                    "refresh_token": refresh, "scope": grant["scope"]})
        if path == "/revoke":
            t = TOKENS.get(f.get("token", ""))
            if t:
                t["active"] = False
                TOKENS[t["pair"]]["active"] = False
                log("revoke: token pair revoked")
            else:
                log("revoke: unknown token (200 per RFC 7009)")
            return self.reply(200, {})
        self.reply(404, {"error": "not_found"})


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--port", type=int, default=8765)
    ap.add_argument("--decision", choices=["allow", "deny"], default="allow")
    ap.add_argument("--deny", action="store_true", help="deny every authorization request")
    ap.add_argument("--log", help="append issuer request events to this file")
    args = ap.parse_args()
    global LOG_PATH
    LOG_PATH = args.log
    Handler.decision = "deny" if args.deny else args.decision
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    log(f"listening on http://127.0.0.1:{server.server_port} decision={args.decision}")
    server.serve_forever()


if __name__ == "__main__":
    main()
