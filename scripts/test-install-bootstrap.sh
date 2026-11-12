#!/usr/bin/env bash
# Resolves the public install.sh tag against a local release API.
# PLEXI_RELEASES_URL replaces api.github.com; the downloaded "installer" records its argv.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
root="$(mktemp -d)"
hits="$root/hits"
argv="$root/argv"
port_file="$root/port"
pid=""
fail=0

cleanup() {
  if [[ -n "$pid" ]]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -rf "$root"
}
trap cleanup EXIT

python3 - "$port_file" "$hits" <<'PY' &
import hashlib, json, sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

port_file, hit_log = sys.argv[1], sys.argv[2]
INSTALLER = b"""#!/bin/sh
: > "$PLEXI_BOOTSTRAP_ARGV_OUT"
for arg in "$@"; do
  printf '%s\\n' "$arg" >> "$PLEXI_BOOTSTRAP_ARGV_OUT"
done
"""
DIGEST = hashlib.sha256(INSTALLER).hexdigest()

def rel(tag, draft=False, linux=True):
    assets = [{"name": "plexi-installer-linux-x64"}] if linux else []
    return {"tag_name": tag, "draft": draft, "prerelease": "-" in tag.split(".", 3)[-1] or tag.count("-") > 0, "assets": assets}

def stables(n):
    return [rel(f"v0.0.{i}") for i in range(1, n + 1)]

# Field order matches the GitHub API: tag_name, then draft.
FIXTURES = {
    "reported": {
        "latest": "v0.3.5",
        "releases": [
            rel("v0.3.5"),
            rel("v0.3.4"),
            rel("v0.3.5-alpha.4"),
            rel("v0.3.5-alpha.1"),
            rel("v0.3.1-alpha.10"),
            rel("v0.3.2-beta.1"),
            rel("v0.3.1-windows.6"),
            rel("v0.3.1-binary.1"),
        ],
    },
    "ordering": {
        "latest": "v0.9.0",
        "releases": [
            rel("v0.9.0"),
            rel("v0.3.5-alpha.2"),
            rel("v0.3.5-alpha.10"),
            rel("v0.3.5-alpha.11", draft=True),
            rel("v0.9.1-beta.1"),
            rel("v0.9.0-beta.4"),
            rel("v0.3.1-windows.2"),
        ],
    },
    "fallback": {
        "latest": "v0.9.0",
        "releases": [
            rel("v0.9.0"),
            rel("v0.4.0-beta.2"),
            rel("v0.3.2-beta.1"),
        ],
    },
    "stable-only": {
        "latest": "v0.3.5",
        "releases": [rel("v0.3.5"), rel("v0.3.4")],
    },
    "stable-asset-fallback": {
        "latest": "v0.4.0",
        "releases": [rel("v0.4.0", linux=False), rel("v0.3.9")],
    },
    "same-series-beta": {
        "latest": "v0.3.5",
        "releases": [
            rel("v0.3.5"),
            rel("v0.3.5-beta.3"),
            rel("v0.3.5-alpha.4"),
            rel("v0.3.2-beta.9"),
        ],
    },
    "paged": {
        "latest": "v0.3.5",
        # 100 releases on page 1 so a single page cannot see alpha.10.
        "releases": [rel("v0.3.5"), rel("v0.3.5-alpha.4")] + stables(98) + [
            rel("v0.3.5-alpha.10"),
            rel("v0.3.5-alpha.11", draft=True),
            rel("v0.3.6-beta.1"),
            rel("v0.3.1-windows.6"),
        ],
    },
    "alpha-linux-fallback": {
        "latest": "v0.4.0",
        "releases": [
            rel("v0.4.0-alpha.2", linux=False),
            rel("v0.3.9-alpha.8"),
            rel("v0.4.0"),
        ],
    },
    "no-linux-assets": {
        "latest": "v0.4.0",
        "releases": [rel("v0.4.0", linux=False), rel("v0.4.0-alpha.2", linux=False)],
    },
}

class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        with open(hit_log, "a", encoding="utf-8") as fh:
            fh.write(self.path + " " + self.headers.get("Authorization", "") + "\n")

    def _send(self, body, content_type="application/json"):
        data = body if isinstance(body, bytes) else body.encode()
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        parsed = urlparse(self.path)
        parts = [p for p in parsed.path.split("/") if p]
        with open(hit_log, "a", encoding="utf-8") as fh:
            fh.write("AUTH " + self.headers.get("Authorization", "") + "\n")
        if parts[:2] == ["fixtures", "rate-limit"]:
            self.send_response(403)
            self.send_header("x-ratelimit-remaining", "0")
            self.send_header("x-ratelimit-reset", "1700000060")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        if parts[:1] == ["download"] and len(parts) == 3:
            name = parts[2]
            if name.endswith(".sha256"):
                asset = name[: -len(".sha256")]
                self._send(f"{DIGEST}  {asset}\n", "text/plain")
            else:
                self._send(INSTALLER, "application/octet-stream")
            return
        if len(parts) >= 2 and parts[0] == "fixtures":
            fixture = FIXTURES.get(parts[1])
            if fixture is None:
                self.send_error(404)
                return
            if parts[-1] == "latest":
                tag = fixture["latest"]
                body = next(r for r in fixture["releases"] if r["tag_name"] == tag and not r["draft"])
                self._send(json.dumps(body, separators=(",", ":")))
                return
            qs = parse_qs(parsed.query)
            per_page = int(qs.get("per_page", ["30"])[0])
            page = int(qs.get("page", ["1"])[0])
            start = (page - 1) * per_page
            chunk = fixture["releases"][start : start + per_page]
            self._send(json.dumps(chunk, separators=(",", ":")))
            return
        self.send_error(404)

server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
with open(port_file, "w", encoding="utf-8") as fh:
    fh.write(str(server.server_address[1]))
server.serve_forever()
PY
pid=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do
  [[ -s "$port_file" ]] && break
  sleep 0.05
done
[[ -s "$port_file" ]] || { echo "fixture server did not start" >&2; exit 1; }
base="http://127.0.0.1:$(cat "$port_file")"

# name fixture expected_tag bootstrap_tag expected_argv -- install args
run_case() {
  local name="$1" fixture="$2" expected_tag="$3" bootstrap_tag="$4" expected_argv="$5"
  shift 5
  : >"$hits"
  : >"$argv"
  local err="" status=0
  if [[ -n "$bootstrap_tag" ]]; then
    err="$(env -u PLEXI_CHANNEL PLEXI_BOOTSTRAP_TAG="$bootstrap_tag" PLEXI_RELEASES_URL="$base/fixtures/$fixture" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" "$@" 2>&1)" || status=$?
  else
    err="$(env -u PLEXI_CHANNEL -u PLEXI_BOOTSTRAP_TAG PLEXI_RELEASES_URL="$base/fixtures/$fixture" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" "$@" 2>&1)" || status=$?
  fi
  local got="" argv_got="" expected_channel=stable previous_arg="" forward_resolved_tag=1
  for arg in "$@"; do
    if [[ "$previous_arg" == "--channel" ]]; then
      expected_channel="$arg"
    elif [[ "$arg" == --channel=* ]]; then
      expected_channel="${arg#--channel=}"
    fi
    previous_arg="$arg"
  done
  [[ "$expected_channel" == pr-* ]] && forward_resolved_tag=0
  got="$(sed -nE 's#.*/download/([^/]+)/.*#\1#p' "$hits" | sort -u | paste -sd, -)"
  argv_got="$(cat "$argv")"
  if (( forward_resolved_tag )) && [[ "$expected_argv" != *$'--tag\n'* && "$expected_argv" != *'--tag='* ]]; then
    if [[ -n "$expected_argv" ]]; then
      expected_argv+=$'\n'
    fi
    expected_argv+="--tag"$'\n'"$expected_tag"
  fi
  if [[ "$status" -ne 0 || "$got" != "$expected_tag" || "$argv_got" != "$expected_argv" ]]; then
    echo "FAIL $name"
    echo "  exit=$status tag='$got' expected='$expected_tag'"
    echo "  argv=$(printf %q "$argv_got") expected=$(printf %q "$expected_argv")"
    [[ -n "$err" ]] && echo "  stderr=$err"
    fail=1
  else
    echo "PASS $name -> $got"
  fi
}

run_case "default uses latest stable" reported v0.3.5 "" ""
run_case "channel stable uses latest stable" reported v0.3.5 "" "--channel
stable" --channel stable
run_case "stable ignores newer beta" ordering v0.9.0 "" "--channel
stable" --channel stable
run_case "stable skips newest release without a Linux installer" stable-asset-fallback v0.3.9 "" "--channel
stable" --channel stable
run_case "channel main uses latest stable" reported v0.3.5 "" "--channel
main" --channel main
run_case "main ignores newer beta" ordering v0.9.0 "" "--channel
main" --channel main
run_case "reported alpha bug" reported v0.3.5-alpha.4 "" "--channel
alpha" --channel alpha
run_case "alpha equals form" reported v0.3.5-alpha.4 "" "--channel=alpha" --channel=alpha
run_case "alpha forwards remaining installer args" reported v0.3.5-alpha.4 "" "--install-only
--channel
alpha" --install-only --channel alpha
run_case "user tag is not duplicated" reported v0.3.5 "" "--tag
v0.3.5-alpha.4" --tag v0.3.5-alpha.4
run_case "beta falls back to newer stable" reported v0.3.5 "" "--channel
beta" --channel beta
run_case "bootstrap tag overrides channel" reported v0.3.5-alpha.1 v0.3.5-alpha.1 "--channel
beta" --channel beta
run_case "alpha prefers newest alpha over a newer stable" ordering v0.3.5-alpha.10 "" "--channel
alpha" --channel alpha
run_case "beta prefers newer beta over stable" ordering v0.9.1-beta.1 "" "--channel
beta" --channel beta
run_case "alpha falls back to newest beta" fallback v0.4.0-beta.2 "" "--channel
alpha" --channel alpha
run_case "beta uses newest stable when it outranks betas" fallback v0.9.0 "" "--channel
beta" --channel beta
run_case "alpha falls back to stable" stable-only v0.3.5 "" "--channel
alpha" --channel alpha
run_case "beta falls back to stable" stable-only v0.3.5 "" "--channel
beta" --channel beta
run_case "beta does not prefer same-series prerelease over stable" same-series-beta v0.3.5 "" "--channel
beta" --channel beta
run_case "alpha reads later release pages" paged v0.3.5-alpha.10 "" "--channel
alpha" --channel alpha
run_case "beta reads later release pages" paged v0.3.6-beta.1 "" "--channel
beta" --channel beta
run_case "pr channel keeps stable installer tag out of package argv" reported v0.3.5 "" "--channel
pr-12" --channel pr-12
run_case "pr channel ignores newer beta" ordering v0.9.0 "" "--channel
pr-12" --channel pr-12
run_case "alpha skips newest release without a Linux installer" alpha-linux-fallback v0.3.9-alpha.8 "" "--channel
alpha" --channel alpha

if err="$(env -u PLEXI_BOOTSTRAP_TAG PLEXI_RELEASES_URL="$base/fixtures/no-linux-assets" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" --channel alpha 2>&1)"; then
  echo "FAIL no Linux installer release was accepted"
  fail=1
elif [[ "$err" != *"No alpha release of Plexi is published for linux-x64 yet."* ]]; then
  echo "FAIL no Linux installer stderr: $err"
  fail=1
else
  echo "PASS no Linux installer release is clear"
fi

if err="$(env -u PLEXI_BOOTSTRAP_TAG PLEXI_RELEASES_URL="$base/fixtures/reported" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" --channel alph 2>&1)"; then
  echo "FAIL unsupported channel was accepted"
  fail=1
elif [[ "$err" != *"Unsupported install channel: alph"* ]]; then
  echo "FAIL unsupported channel stderr: $err"
  fail=1
else
  echo "PASS unsupported channel is rejected"
fi

: >"$hits"
if ! env -u PLEXI_BOOTSTRAP_TAG GITHUB_TOKEN=bootstrap-test-token PLEXI_RELEASES_URL="$base/fixtures/reported" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" --channel alpha >/dev/null 2>&1; then
  echo "FAIL bootstrap token fixture"
  fail=1
elif ! grep -Fq 'AUTH Bearer bootstrap-test-token' "$hits"; then
  echo "FAIL bootstrap did not send GITHUB_TOKEN"
  fail=1
else
  echo "PASS bootstrap sends GITHUB_TOKEN"
fi

if err="$(env -u PLEXI_BOOTSTRAP_TAG PLEXI_RELEASES_URL="$base/fixtures/rate-limit" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" --channel alpha 2>&1)"; then
  echo "FAIL rate-limited bootstrap succeeded"
  fail=1
elif [[ "$err" != *"GitHub API rate limit reached"* || "$err" != *"GITHUB_TOKEN"* || "$err" == *"No alpha release of Plexi is published for linux-x64 yet."* ]]; then
  echo "FAIL rate-limit bootstrap stderr: $err"
  fail=1
else
  echo "PASS rate-limit bootstrap is clear"
fi

if [[ "$fail" -ne 0 ]]; then
  echo "install bootstrap channel selection failed"
  exit 1
fi
echo "install bootstrap channel selection passed"
