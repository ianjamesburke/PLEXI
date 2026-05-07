#!/usr/bin/env bash
# Install Plexi from GitHub releases.
# Usage: curl -fsSL https://plexiapp.com/install | sh
set -euo pipefail

REPO="ianjamesburke/PLEXI"
APP_NAME="Plexi.app"
APP_DEST="/Applications/Plexi.app"
CLI_DIR="/usr/local/bin"
CLI_DEST="$CLI_DIR/plexi"

# ── helpers ──────────────────────────────────────────────────────────────────

info()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
ok()    { printf '\033[1;32m ✓\033[0m %s\n' "$*"; }
warn()  { printf '\033[1;33mwarn:\033[0m %s\n' "$*" >&2; }
die()   { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

require() {
    command -v "$1" &>/dev/null || die "required tool not found: $1"
}

# ── preflight ─────────────────────────────────────────────────────────────────

[[ "$(uname)" == "Darwin" ]] || die "Plexi is macOS only."
require curl
require unzip

# ── fetch latest release ──────────────────────────────────────────────────────

info "Fetching latest release..."
API="https://api.github.com/repos/$REPO/releases/latest"
TAG=$(curl -fsSL "$API" | grep '"tag_name"' | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
[[ -n "$TAG" ]] || die "Could not determine latest release tag."

ZIP_NAME="Plexi-${TAG}.zip"
DOWNLOAD_URL="https://github.com/$REPO/releases/download/$TAG/$ZIP_NAME"

info "Downloading $ZIP_NAME..."
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
curl -fsSL --progress-bar "$DOWNLOAD_URL" -o "$TMP/$ZIP_NAME"

# ── install app bundle ─────────────────────────────────────────────────────────

info "Installing $APP_NAME to /Applications..."
unzip -q "$TMP/$ZIP_NAME" -d "$TMP/extracted"

# cargo-bundle produces a lowercase bundle name on disk; find it regardless.
APP_SRC=$(find "$TMP/extracted" -maxdepth 1 -name "*.app" | head -1)
[[ -n "$APP_SRC" ]] || die "No .app found inside $ZIP_NAME."

rm -rf "$APP_DEST"
cp -R "$APP_SRC" "$APP_DEST"
ok "Installed $APP_DEST"

# ── CLI symlink ────────────────────────────────────────────────────────────────

info "Setting up CLI..."
BINARY="$APP_DEST/Contents/MacOS/plexi"
[[ -f "$BINARY" ]] || die "Binary not found at $BINARY."

if [[ ! -d "$CLI_DIR" ]]; then
    sudo mkdir -p "$CLI_DIR"
fi
sudo ln -sf "$BINARY" "$CLI_DEST"
ok "CLI: $CLI_DEST"

# ── shell integration ─────────────────────────────────────────────────────────

info "Setting up shell integration..."
SNIPPET='eval "$(plexi shell-init)"'

for RC in "$HOME/.zshrc" "$HOME/.bashrc"; do
    [[ -f "$RC" ]] || continue
    if ! grep -qF 'plexi shell-init' "$RC"; then
        printf '\n# Plexi shell integration\n%s\n' "$SNIPPET" >> "$RC"
        ok "Added shell integration to $RC"
    else
        ok "Shell integration already present in $RC"
    fi
done

# ── register with macOS (dock icon, Spotlight, Open With) ─────────────────────

info "Registering app with macOS..."
LSREG="/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister"
if [[ -x "$LSREG" ]]; then
    "$LSREG" -f "$APP_DEST" 2>/dev/null && ok "Registered with LaunchServices"
fi
/System/Library/CoreServices/pbs -update 2>/dev/null || true

# ── done ──────────────────────────────────────────────────────────────────────

echo ""
echo "  Plexi $TAG installed."
echo ""
echo "  Open it:  open /Applications/Plexi.app"
echo "  CLI:      plexi --version  (after restarting your terminal)"
echo ""
echo "  Or reload now:  source ~/.zshrc"
