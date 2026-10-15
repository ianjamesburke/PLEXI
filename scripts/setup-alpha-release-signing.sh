#!/usr/bin/env bash
# Import the one persistent self-signed identity used for public Alpha packages.
# Secrets are supplied only by the release workflow; this script never creates
# or substitutes an identity because doing so would break Keychain continuity.
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
  echo "error: Alpha release signing is macOS-only" >&2
  exit 1
fi

: "${PLEXI_ALPHA_SIGNING_P12_BASE64:?missing protected Alpha signing certificate}"
: "${PLEXI_ALPHA_SIGNING_P12_PASSWORD:?missing protected Alpha signing certificate password}"
: "${PLEXI_ALPHA_SIGNING_CERT_SHA1:?missing pinned Alpha signing certificate SHA-1}"
: "${RUNNER_TEMP:?GitHub runner temporary directory is required}"
: "${GITHUB_ENV:?GitHub environment file is required}"

keychain="$RUNNER_TEMP/plexi-alpha-release-signing.keychain-db"
p12="$RUNNER_TEMP/plexi-alpha-release-signing.p12"
certificate="$RUNNER_TEMP/plexi-alpha-release-signing.pem"
cleanup() { rm -f "$p12"; }
trap cleanup EXIT

uv run --no-project python - <<'PY' >"$p12"
import base64, os, sys
sys.stdout.buffer.write(base64.b64decode(os.environ["PLEXI_ALPHA_SIGNING_P12_BASE64"], validate=True))
PY

security create-keychain -p "$PLEXI_ALPHA_SIGNING_P12_PASSWORD" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$PLEXI_ALPHA_SIGNING_P12_PASSWORD" "$keychain"
security import "$p12" -k "$keychain" -P "$PLEXI_ALPHA_SIGNING_P12_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple: -s -k "$PLEXI_ALPHA_SIGNING_P12_PASSWORD" "$keychain"

expected="$(printf '%s' "$PLEXI_ALPHA_SIGNING_CERT_SHA1" | tr '[:lower:]' '[:upper:]' | tr -d ':')"
security find-certificate -c 'Plexi Dev' -p "$keychain" >"$certificate"
actual="$(openssl x509 -in "$certificate" -noout -fingerprint -sha1 | sed 's/.*=//' | tr -d ':' | tr '[:lower:]' '[:upper:]')"
if [[ "$actual" != "$expected" ]]; then
  echo "error: imported Alpha signing certificate does not match PLEXI_ALPHA_SIGNING_CERT_SHA1" >&2
  exit 1
fi

# A self-signed certificate needs code-signing trust on the disposable runner.
# Pin it before adding trust; the workflow removes this setting at teardown.
security add-trusted-cert -r trustRoot -p codeSign -k "$keychain" "$certificate"
identity="$(security find-identity -v -p codesigning "$keychain" | awk -v wanted="$expected" 'toupper($2) == wanted { print $2; exit }')"
if [[ -z "$identity" ]]; then
  echo "error: protected Alpha certificate is not a usable code-signing identity" >&2
  exit 1
fi
printf 'PLEXI_SIGN_IDENTITY=%s\n' "$identity" >>"$GITHUB_ENV"
printf 'PLEXI_SIGNING_CERT_SHA1=%s\n' "$expected" >>"$GITHUB_ENV"
printf 'PLEXI_SIGNING_KEYCHAIN=%s\n' "$keychain" >>"$GITHUB_ENV"
echo "release-signing: imported persistent Alpha certificate sha1=$expected"
