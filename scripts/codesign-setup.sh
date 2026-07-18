#!/usr/bin/env bash
# One-time: create a stable self-signed code-signing identity named "Plexi Dev"
# in the login keychain. scripts/install.sh signs the installed bundle with this
# identity so its code signature's designated requirement pins to the cert
# (stable) instead of a per-build cdhash (ad-hoc). macOS keys keychain "Always
# Allow" ACLs off the designated requirement, so an ad-hoc bundle re-prompts for
# keychain access after every rebuild — that is why the AI broker's
# OPENROUTER_API_KEY read (service "plexi") re-asks each install. A stable
# identity fixes the requirement and stops the re-prompt.
#
# Idempotent: exits 0 if a valid "Plexi Dev" codesigning identity already exists.
set -euo pipefail

if [[ "$(uname)" != "Darwin" ]]; then
  echo "codesign-setup is macOS-only."
  exit 1
fi

IDENTITY="Plexi Dev"
LOGIN_KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"

if security find-identity -v -p codesigning 2>/dev/null | grep -q "$IDENTITY"; then
  echo "Code-signing identity '$IDENTITY' already present — nothing to do."
  exit 0
fi

echo "Creating self-signed code-signing identity '$IDENTITY'..."

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

key="$workdir/plexi-dev.key"
cert="$workdir/plexi-dev.crt"
p12="$workdir/plexi-dev.p12"
cfg="$workdir/openssl.cnf"
# Transient p12 password: the file lives only inside $workdir and is deleted on
# exit. A random value keeps it off the process table in a predictable form.
p12_pass="$(openssl rand -hex 16)"

# codeSigning EKU + the digitalSignature key usage codesign requires. CA:false
# self-signed leaf, used directly as the signing identity.
cat > "$cfg" <<'CNF'
[ req ]
distinguished_name = dn
x509_extensions    = v3_codesign
prompt             = no

[ dn ]
CN = Plexi Dev

[ v3_codesign ]
basicConstraints   = critical, CA:false
keyUsage           = critical, digitalSignature
extendedKeyUsage   = critical, codeSigning
CNF

openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout "$key" -out "$cert" \
  -days 3650 -config "$cfg"

# -legacy is required: OpenSSL 3.x defaults to a SHA-256 PKCS#12 MAC that Apple's
# `security import` cannot verify ("MAC verification failed"). -legacy emits the
# RC2/3DES + SHA-1 MAC the security framework expects.
openssl pkcs12 -export -legacy -out "$p12" \
  -inkey "$key" -in "$cert" \
  -name "$IDENTITY" -passout "pass:$p12_pass"

# -A grants every app access to the private key so codesign never prompts for
# keychain access at sign time.
security import "$p12" -k "$LOGIN_KEYCHAIN" -P "$p12_pass" -A

echo ""
echo ">>> macOS may now ask you to authorize trusting '$IDENTITY' for code signing."
echo ">>> If a dialog appears, enter your login password (one-time)."
echo ""

# Trust the self-signed cert for code signing only. Without this the identity
# evaluates as CSSMERR_TP_NOT_TRUSTED and codesign refuses to use it. This
# writes a user-domain trust setting and MAY pop one authorization dialog.
security add-trusted-cert -r trustRoot -p codeSign "$cert"

if security find-identity -v -p codesigning 2>/dev/null | grep -q "$IDENTITY"; then
  echo "Success: '$IDENTITY' is now a valid code-signing identity."
  echo "Run 'just install' — it will sign the bundle with this identity."
else
  echo "Error: '$IDENTITY' was created but is not a valid codesigning identity."
  echo "  Inspect: security find-identity -v -p codesigning"
  exit 1
fi
