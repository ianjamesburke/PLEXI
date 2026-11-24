#!/usr/bin/env bash
# Resolves the public install.sh tag against a local release API.
# PLEXI_GITHUB_REPO_API replaces api.github.com; the downloaded "installer" records its argv.
set -euo pipefail

# No real credential enters the harness. Inherited tokens are kept unexported,
# only to scan the buffered output for them before it is released.
inherited_tokens=()
for var in GITHUB_TOKEN GH_TOKEN; do
  [[ -n "${!var:-}" ]] && inherited_tokens+=("${!var}")
done
unset GITHUB_TOKEN GH_TOKEN
dummy_token="bootstrap-test-token"

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
root="$(mktemp -d)"
hits="$root/hits"
argv="$root/argv"
port_file="$root/port"
out="$root/out"
pid=""
fail=0
case "$(uname -s)" in Darwin) host_os=macos ;; *) host_os=linux ;; esac
case "$(uname -m)" in arm64|aarch64) host_arch=arm64 ;; *) host_arch=x64 ;; esac
platform="$host_os-$host_arch"

# True when file $1 contains any of the remaining arguments.
contains_any() {
  local text token
  text="$(<"$1")"
  shift
  for token in "$@"; do
    [[ -n "$token" && "$text" == *"$token"* ]] && return 0
  done
  return 1
}

# All output is buffered and released only if it holds no inherited token.
exec 3>&1 4>&2 >"$out" 2>&1
cleanup() {
  local rc=$?
  if [[ -n "$pid" ]]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  exec 1>&3 2>&4
  if (( ${#inherited_tokens[@]} )) && contains_any "$out" "${inherited_tokens[@]}"; then
    echo "FAIL harness output contains an inherited GITHUB_TOKEN or GH_TOKEN value; output withheld"
    rc=1
  else
    cat "$out"
  fi
  rm -rf "$root"
  exit "$rc"
}
trap cleanup EXIT

python3 - "$port_file" "$hits" "plexi-installer-$platform" "$dummy_token" <<'PY' &
import hashlib, json, sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

port_file, hit_log, installer_asset, dummy_token = sys.argv[1:5]
INSTALLER = b"""#!/bin/sh
: > "$PLEXI_BOOTSTRAP_ARGV_OUT"
for arg in "$@"; do
  printf '%s\\n' "$arg" >> "$PLEXI_BOOTSTRAP_ARGV_OUT"
done
"""
DIGEST = hashlib.sha256(INSTALLER).hexdigest()

def rel(tag, draft=False, linux=True):
    assets = [{"name": installer_asset}] if linux else []
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

# 504s served before each fixture answers; "down" never answers.
FAILURES = {"flaky": 1, "down": 10**9}
FIXTURES["flaky"] = FIXTURES["reported"]
FIXTURES["down"] = FIXTURES["reported"]

# A port nothing listens on: bound once for a number, then closed.
import socket
_probe = socket.socket()
_probe.bind(("127.0.0.1", 0))
CLOSED_PORT = _probe.getsockname()[1]
_probe.close()

class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        with open(hit_log, "a", encoding="utf-8") as fh:
            fh.write(self.path + "\n")

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
        # Records which credential arrived, never its value.
        auth = self.headers.get("Authorization")
        kind = "none" if auth is None else "dummy" if auth == "Bearer " + dummy_token else "other"
        with open(hit_log, "a", encoding="utf-8") as fh:
            fh.write("AUTH " + kind + "\n")
        if parts[:2] == ["fixtures", "truncated"]:
            self.send_response(504)
            self.send_header("Content-Length", "100")
            self.end_headers()
            self.wfile.write(b"short")
            self.wfile.flush()
            self.close_connection = True
            return
        if parts[:2] == ["fixtures", "redirect-closed"]:
            self.send_response(302)
            self.send_header("Location", f"http://127.0.0.1:{CLOSED_PORT}/")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
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
            rest = parts[2:]
            failures = FAILURES.get(parts[1], 0)
            if failures:
                FAILURES[parts[1]] = failures - 1
                self.send_response(504)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            published = [r for r in fixture["releases"] if not r["draft"]]
            if rest == ["releases", "latest"]:
                body = next(r for r in published if r["tag_name"] == fixture["latest"])
                self._send(json.dumps(body, separators=(",", ":")))
                return
            if rest == ["git", "matching-refs", "tags", "v"]:
                refs = [{"ref": "refs/tags/" + r["tag_name"]} for r in fixture["releases"]]
                self._send(json.dumps(refs, separators=(",", ":")))
                return
            if rest[:2] == ["releases", "tags"] and len(rest) == 3:
                body = next((r for r in published if r["tag_name"] == rest[2]), None)
                if body is None:
                    self.send_error(404)
                else:
                    self._send(json.dumps(body, separators=(",", ":")))
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
    err="$(env -u PLEXI_CHANNEL PLEXI_BOOTSTRAP_TAG="$bootstrap_tag" PLEXI_GITHUB_REPO_API="$base/fixtures/$fixture" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" "$@" 2>&1)" || status=$?
  else
    err="$(env -u PLEXI_CHANNEL -u PLEXI_BOOTSTRAP_TAG PLEXI_GITHUB_REPO_API="$base/fixtures/$fixture" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" "$@" 2>&1)" || status=$?
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
run_case "user tag pins the installer and is not duplicated" reported v0.3.5-alpha.4 "" "--tag
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
run_case "alpha sees tags past the newest hundred releases" paged v0.3.5-alpha.10 "" "--channel
alpha" --channel alpha
run_case "beta sees tags past the newest hundred releases" paged v0.3.6-beta.1 "" "--channel
beta" --channel beta
run_case "pr channel keeps stable installer tag out of package argv" reported v0.3.5 "" "--channel
pr-12" --channel pr-12
run_case "pr channel ignores newer beta" ordering v0.9.0 "" "--channel
pr-12" --channel pr-12
run_case "alpha skips newest release without this platform's installer" alpha-linux-fallback v0.3.9-alpha.8 "" "--channel
alpha" --channel alpha

if err="$(env -u PLEXI_BOOTSTRAP_TAG PLEXI_GITHUB_REPO_API="$base/fixtures/no-linux-assets" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" --channel alpha 2>&1)"; then
  echo "FAIL no Linux installer release was accepted"
  fail=1
elif [[ "$err" != *"No alpha release of Plexi is published for $platform yet."* ]]; then
  echo "FAIL no Linux installer stderr: $err"
  fail=1
else
  echo "PASS no Linux installer release is clear"
fi

if err="$(env -u PLEXI_BOOTSTRAP_TAG PLEXI_GITHUB_REPO_API="$base/fixtures/reported" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" --channel alph 2>&1)"; then
  echo "FAIL unsupported channel was accepted"
  fail=1
elif [[ "$err" != *"Unsupported install channel: alph"* ]]; then
  echo "FAIL unsupported channel stderr: $err"
  fail=1
else
  echo "PASS unsupported channel is rejected"
fi

: >"$hits"
if ! env -u PLEXI_BOOTSTRAP_TAG GITHUB_TOKEN="$dummy_token" PLEXI_GITHUB_REPO_API="$base/fixtures/reported" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" --channel alpha >/dev/null 2>&1; then
  echo "FAIL bootstrap token fixture"
  fail=1
elif ! grep -Fxq 'AUTH dummy' "$hits"; then
  echo "FAIL bootstrap did not send GITHUB_TOKEN"
  fail=1
else
  echo "PASS bootstrap sends GITHUB_TOKEN"
fi

: >"$hits"
env -u PLEXI_BOOTSTRAP_TAG PLEXI_GITHUB_REPO_API="$base/fixtures/reported" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" --channel alpha >/dev/null 2>&1 || true
if grep -Eq '^AUTH (dummy|other)$' "$hits" || ! grep -Fxq 'AUTH none' "$hits"; then
  echo "FAIL bootstrap sent a credential the harness did not set"
  fail=1
else
  echo "PASS inherited GITHUB_TOKEN and GH_TOKEN never reach the bootstrap"
fi

# The output gate in cleanup: a line holding a token is caught, a clean one is not.
printf '/fixtures/reported/releases Bearer leak-sentinel-1711\n' >"$root/leak"
printf '/fixtures/reported/releases\n' >"$root/clean"
if contains_any "$root/leak" leak-sentinel-1711 && ! contains_any "$root/clean" leak-sentinel-1711 && ! contains_any "$root/clean" ""; then
  echo "PASS output gate catches a token line"
else
  echo "FAIL output gate missed a token line"
  fail=1
fi

if err="$(env -u PLEXI_BOOTSTRAP_TAG PLEXI_GITHUB_REPO_API="$base/fixtures/rate-limit" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" --channel alpha 2>&1)"; then
  echo "FAIL rate-limited bootstrap succeeded"
  fail=1
elif [[ "$err" != *"GitHub API rate limit reached"* || "$err" != *"GITHUB_TOKEN"* || "$err" == *"No alpha release of Plexi is published for $platform yet."* ]]; then
  echo "FAIL rate-limit bootstrap stderr: $err"
  fail=1
else
  echo "PASS rate-limit bootstrap is clear"
fi

run_case "flaky GitHub is retried" flaky v0.3.5-alpha.4 "" "--channel
alpha" --channel alpha

: >"$hits"
for args in "--channel alpha" "--channel beta" "--channel stable" "--tag v0.3.5-alpha.4"; do
  # shellcheck disable=SC2086
  env -u PLEXI_BOOTSTRAP_TAG PLEXI_GITHUB_REPO_API="$base/fixtures/reported" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" $args >/dev/null 2>&1 || true
done
if grep -Eq '^/fixtures/[^ ]+/releases(\?[^ ]*)?$' "$hits"; then
  echo "FAIL bootstrap read the paginated release list"
  grep -E '^/fixtures/[^ ]+/releases(\?[^ ]*)?$' "$hits" | head -3
  fail=1
else
  echo "PASS bootstrap never reads the paginated release list"
fi

: >"$hits"
env -u PLEXI_BOOTSTRAP_TAG PLEXI_GITHUB_REPO_API="$base/fixtures/reported" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" --tag v0.3.5-alpha.4 >/dev/null 2>&1 || true
if grep -q '^/fixtures/' "$hits"; then
  echo "FAIL --tag queried the release API"
  fail=1
else
  echo "PASS --tag makes no release API request"
fi

if err="$(env -u PLEXI_BOOTSTRAP_TAG PLEXI_GITHUB_REPO_API="$base/fixtures/stable-asset-fallback" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" --channel stable 2>&1)"; then
  echo "FAIL stable installed a release older than GitHub's latest"
  fail=1
elif [[ "$err" != *"No stable release of Plexi is published for $platform yet."* ]]; then
  echo "FAIL stable incomplete latest stderr: $err"
  fail=1
else
  echo "PASS stable uses only GitHub's latest release"
fi

: >"$hits"
started=$SECONDS
if err="$(env -u PLEXI_BOOTSTRAP_TAG PLEXI_GITHUB_REPO_API="$base/fixtures/down" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" --channel alpha 2>&1)"; then
  echo "FAIL persistent 504 bootstrap succeeded"
  fail=1
elif [[ "$err" != *"GitHub is slow, try again."* || "$err" != *"HTTP 504"* ]]; then
  echo "FAIL persistent 504 stderr: $err"
  fail=1
elif [[ "$(grep -c '^/fixtures/down/' "$hits")" != 3 ]] || (( SECONDS - started > 10 )); then
  echo "FAIL persistent 504 was not bounded to 3 attempts: $(grep -c '^/fixtures/down/' "$hits") requests in $((SECONDS - started))s"
  fail=1
else
  echo "PASS persistent 504 stops after 3 attempts and says GitHub is slow"
fi

: >"$hits"
if env -u PLEXI_BOOTSTRAP_TAG PLEXI_GITHUB_REPO_API="$base/fixtures/rate-limit" PLEXI_RELEASE_BASE_URL="$base/download" PLEXI_BOOTSTRAP_ARGV_OUT="$argv" bash "$repo/install.sh" --channel alpha >/dev/null 2>&1; then
  echo "FAIL rate-limited bootstrap succeeded"
  fail=1
elif [[ "$(grep -c '^/fixtures/rate-limit/' "$hits")" != 1 ]]; then
  echo "FAIL rate limit was retried: $(grep -c '^/fixtures/rate-limit/' "$hits") requests"
  fail=1
else
  echo "PASS rate limit is not retried"
fi

# classify_response, cell by cell, through a fake curl: it writes the row's
# status and extra header (no header file for 000), prints %{http_code} and
# exits with the row's code. A no-op sleep keeps retry rows instant. Rows
# follow the table in the PR #3292 design: status rc header calls expected.
# expected: "rate" = rate-limit message, "slow" = GitHub is slow, "fail" = an
# immediate failure that is not "slow", otherwise the HTTP status the caller
# reports.
mkdir -p "$root/fakebin" "$root/fakesleep"
cat >"$root/fakebin/curl" <<'SH'
#!/bin/sh
echo call >>"$FAKE_CURL_CALLS"
headers="" previous=""
for arg in "$@"; do
  [ "$previous" = "-D" ] && headers="$arg"
  previous="$arg"
done
if [ "$FAKE_CURL_STATUS" != 000 ]; then
  { printf 'HTTP/1.1 %s\r\n' "$FAKE_CURL_STATUS"; [ "$FAKE_CURL_HEADER" != - ] && printf '%s\r\n' "$(printf '%s' "$FAKE_CURL_HEADER" | sed 's/:/: /')"; printf '\r\n'; } >"$headers"
fi
printf '%s' "$FAKE_CURL_STATUS"
exit "$FAKE_CURL_EXIT"
SH
printf '#!/bin/sh\nexit 0\n' >"$root/fakesleep/sleep"
chmod +x "$root/fakebin/curl" "$root/fakesleep/sleep"
cp "$root/fakesleep/sleep" "$root/fakebin/sleep"

# Checks one bootstrap run's stderr and request count against a row.
check_row() {
  local name="$1" err="$2" calls_got="$3" calls="$4" expected="$5" ok=1
  case "$expected" in
    rate) [[ "$err" == *"GitHub API rate limit reached"* ]] || ok=0 ;;
    slow) [[ "$err" == *"GitHub is slow, try again."* ]] || ok=0 ;;
    fail) [[ "$err" == *"GitHub request to "*" failed (curl exit "* && "$err" != *"GitHub is slow"* ]] || ok=0 ;;
    *) [[ "$err" == *"Unable to list Plexi release tags: HTTP $expected."* ]] || ok=0 ;;
  esac
  if (( ok )) && [[ "$calls_got" == "$calls" ]]; then
    echo "PASS $name"
  else
    echo "FAIL $name: $calls_got calls (want $calls), stderr: $err"
    fail=1
  fi
}

while read -r status code header calls expected; do
  [[ -z "$status" || "$status" == \#* ]] && continue
  : >"$root/calls"
  err="$(env -u PLEXI_BOOTSTRAP_TAG PATH="$root/fakebin:$PATH" FAKE_CURL_CALLS="$root/calls" FAKE_CURL_STATUS="$status" FAKE_CURL_EXIT="$code" FAKE_CURL_HEADER="$header" PLEXI_GITHUB_REPO_API="$base/fixtures/reported" bash "$repo/install.sh" --channel alpha 2>&1)" && err="unexpected success: $err"
  check_row "curl $status / exit $code ($header)" "$err" "$(wc -l <"$root/calls" | tr -d ' ')" "$calls" "$expected"
done <<'ROWS'
429 0 - 1 rate
429 56 - 1 rate
403 28 x-ratelimit-remaining:0 1 rate
403 0 - 1 403
404 28 - 1 404
504 0 - 3 slow
504 56 - 3 slow
504 18 - 3 slow
504 92 - 3 slow
200 18 - 3 slow
200 28 - 3 slow
200 23 - 1 fail
302 0 - 1 302
302 7 - 3 slow
302 47 - 1 fail
000 7 - 3 slow
000 28 - 3 slow
000 6 - 1 fail
000 60 - 1 fail
000 127 - 1 fail
000 99 - 3 slow
ROWS

# The same cells through real curl: a 504 whose body stops short of its
# Content-Length (curl 504 / exit 18) and a redirect to a closed port
# (curl 302 / exit 7). Both are retried.
for fixture in truncated redirect-closed; do
  : >"$hits"
  err="$(env -u PLEXI_BOOTSTRAP_TAG PATH="$root/fakesleep:$PATH" PLEXI_GITHUB_REPO_API="$base/fixtures/$fixture" bash "$repo/install.sh" --channel alpha 2>&1)" && err="unexpected success: $err"
  check_row "real curl $fixture" "$err" "$(grep -c "^/fixtures/$fixture/" "$hits")" 3 slow
done

if [[ "$fail" -ne 0 ]]; then
  echo "install bootstrap channel selection failed"
  exit 1
fi
echo "install bootstrap channel selection passed"
