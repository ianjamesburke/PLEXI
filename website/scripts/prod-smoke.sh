#!/usr/bin/env bash
# Production smoke check for plexiapp.com (stint 0345).
# Verifies the surfaces the host and first users depend on. Run after every
# deploy: `just website-smoke`. Nonzero exit on any failure.
set -euo pipefail

BASE="${PLEXI_SITE_BASE:-https://plexiapp.com}"
fail=0

check() {
  local label="$1" url="$2" expect="${3:-200}"
  local code
  code=$(curl -s -o /dev/null -w "%{http_code}" -L --max-time 20 "$url") || code="curl-error"
  if [[ "$code" == "$expect" ]]; then
    echo "ok   $label ($code) $url"
  else
    echo "FAIL $label (got $code, want $expect) $url"
    fail=1
  fi
}

check "home page"        "$BASE/"
check "download page"    "$BASE/download"
check "install redirect" "$BASE/install" 200   # -L follows 302 to install.sh
check "registry index"   "$BASE/registry/v1/index.json"

# Every artifact in the index must download and match its checksum name.
index=$(curl -s --max-time 20 "$BASE/registry/v1/index.json")
for checksum in $(echo "$index" | python3 -c "import json,sys; [print(a['checksum']) for a in json.load(sys.stdin)['apps']]"); do
  url="$BASE/registry/v1/packages/$checksum.plexipkg"
  actual=$(curl -s -L --max-time 60 "$url" | shasum -a 256 | cut -d' ' -f1) || actual="download-failed"
  if [[ "$actual" == "$checksum" ]]; then
    echo "ok   artifact $checksum"
  else
    echo "FAIL artifact $checksum (checksum mismatch or download failed: $actual)"
    fail=1
  fi
done

if [[ "$fail" -ne 0 ]]; then
  echo "prod smoke: FAILED against $BASE"
  exit 1
fi
echo "prod smoke: all green against $BASE"
