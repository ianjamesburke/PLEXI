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

# ── fetch version (needed for banner) ────────────────────────────────────────

API="https://api.github.com/repos/$REPO/releases/latest"
TAG=$(curl -fsSL "$API" | grep -m 1 '"tag_name"' | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
[[ -n "$TAG" ]] || die "Could not determine latest release tag."

# ── welcome banner ───────────────────────────────────────────────────────────

echo ""
SUBTITLE="the last app you'll ever need"
BOX_W=$(( ${#SUBTITLE} + 16 ))
echo "  ╔$(printf '═%.0s' $(seq 1 $BOX_W))╗"
TITLE="P L E X I  $TAG"
PAD=$(( (BOX_W - ${#TITLE}) / 2 ))
printf "  ║%*s%s%*s║\n" $PAD "" "$TITLE" $(( BOX_W - PAD - ${#TITLE} )) ""
SUB_PAD=$(( (BOX_W - ${#SUBTITLE}) / 2 ))
printf "  ║%*s%s%*s║\n" $SUB_PAD "" "$SUBTITLE" $(( BOX_W - SUB_PAD - ${#SUBTITLE} )) ""
echo "  ╚$(printf '═%.0s' $(seq 1 $BOX_W))╝"
echo ""
echo "  This installer will:"
echo "    • Download Plexi $TAG to /Applications"
echo "    • Add the CLI to /usr/local/bin/plexi"
echo "    • Install shell completions"
echo "    • Install agent skills for AI coding assistants"
echo "    • Sign the app for macOS Gatekeeper"
echo ""
echo "  Admin access may be required for the CLI symlink."
echo ""
printf "  \033[2mPlexi is early-stage software. If anything\n"
printf "  goes wrong, reach out — happy to help.\033[0m\n"
echo ""
read -r -p "  Press Enter to continue, or Ctrl+C to cancel. " </dev/tty
echo ""

# ── sudo upfront ─────────────────────────────────────────────────────────────

NEED_SUDO=false
if [[ ! -w "$CLI_DIR" ]]; then
    NEED_SUDO=true
fi
if [[ ! -d "$CLI_DIR" ]]; then
    NEED_SUDO=true
fi

if $NEED_SUDO; then
    info "Requesting admin access..."
    sudo -v || die "Admin access is required to install the CLI to $CLI_DIR."
fi

# ── download ─────────────────────────────────────────────────────────────────

ZIP_NAME="Plexi-${TAG}.zip"
DOWNLOAD_URL="https://github.com/$REPO/releases/download/$TAG/$ZIP_NAME"

info "Downloading $ZIP_NAME..."
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
curl -fsSL --progress-bar "$DOWNLOAD_URL" -o "$TMP/$ZIP_NAME"

# ── install app bundle ─────────────────────────────────────────────────────────

info "Installing $APP_NAME to /Applications..."
unzip -q "$TMP/$ZIP_NAME" -d "$TMP/extracted"

APP_SRC=$(find "$TMP/extracted" -maxdepth 1 -name "*.app" | head -1)
[[ -n "$APP_SRC" ]] || die "No .app found inside $ZIP_NAME."

rm -rf "$APP_DEST"
cp -R "$APP_SRC" "$APP_DEST"

# clear Gatekeeper quarantine flag so the app opens without "unidentified developer" block
xattr -c "$APP_DEST" 2>/dev/null || true
ok "Installed $APP_DEST"

# ── CLI symlink ────────────────────────────────────────────────────────────────

info "Setting up CLI..."
BINARY="$APP_DEST/Contents/MacOS/plexi"
[[ -f "$BINARY" ]] || die "Binary not found at $BINARY."

if $NEED_SUDO; then
    [[ -d "$CLI_DIR" ]] || sudo mkdir -p "$CLI_DIR"
    sudo ln -sf "$BINARY" "$CLI_DEST"
else
    [[ -d "$CLI_DIR" ]] || mkdir -p "$CLI_DIR"
    ln -sf "$BINARY" "$CLI_DEST"
fi
ok "CLI: $CLI_DEST"

# ── shell integration ─────────────────────────────────────────────────────────

info "Installing shell completions..."
if command -v brew &>/dev/null; then
    BREW_ZSH="$(brew --prefix)/share/zsh/site-functions"
    if [[ -d "$BREW_ZSH" ]]; then
        "$CLI_DEST" completions zsh > "$BREW_ZSH/_plexi"
        ok "Completions (zsh): $BREW_ZSH/_plexi"
    fi
else
    mkdir -p "$HOME/.zfunc"
    "$CLI_DEST" completions zsh > "$HOME/.zfunc/_plexi"
    ok "Completions (zsh): ~/.zfunc/_plexi"
    # ensure ~/.zfunc is on fpath
    if [[ -f "$HOME/.zshrc" ]] && ! grep -qF '.zfunc' "$HOME/.zshrc"; then
        printf '\nfpath=(~/.zfunc $fpath)\nautoload -Uz compinit && compinit\n' >> "$HOME/.zshrc"
        ok "Added ~/.zfunc to fpath in ~/.zshrc"
    fi
fi
mkdir -p "$HOME/.bash_completion.d"
"$CLI_DEST" completions bash > "$HOME/.bash_completion.d/plexi"
ok "Completions (bash): ~/.bash_completion.d/plexi"
if [[ -d "$HOME/.config/fish" ]]; then
    mkdir -p "$HOME/.config/fish/completions"
    "$CLI_DEST" completions fish > "$HOME/.config/fish/completions/plexi.fish"
    ok "Completions (fish): ~/.config/fish/completions/plexi.fish"
fi

# ── agent skills ──────────────────────────────────────────────────────────────

PROFILE_DIR="$HOME/.plexi"
SKILLS_SRC=$(find "$TMP/extracted" -maxdepth 1 -type d -name "skills" | head -1)
if [[ -n "$SKILLS_SRC" ]]; then
    info "Installing agent skills..."
    SKILLS_DEST="$PROFILE_DIR/.agents/skills"
    installed=0
    for skill_dir in "$SKILLS_SRC"/*/; do
        [[ -f "$skill_dir/SKILL.md" ]] || continue
        name="$(basename "$skill_dir")"
        mkdir -p "$SKILLS_DEST/$name"
        cp -R "$skill_dir"/* "$SKILLS_DEST/$name/"
        installed=$((installed + 1))
    done
    [[ $installed -gt 0 ]] && ok "Skills: $installed installed to $SKILLS_DEST/"
fi

# ── register with macOS (dock icon, Spotlight, Open With) ─────────────────────

info "Registering app with macOS..."
LSREG="/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister"
if [[ -x "$LSREG" ]]; then
    "$LSREG" -f "$APP_DEST" 2>/dev/null && ok "Registered with LaunchServices"
fi
/System/Library/CoreServices/pbs -update 2>/dev/null || true

# ── done ──────────────────────────────────────────────────────────────────────

echo ""
echo "  ╔$(printf '═%.0s' $(seq 1 $BOX_W))╗"
DONE="Plexi $TAG installed."
PAD=$(( (BOX_W - ${#DONE}) / 2 ))
printf "  ║%*s%s%*s║\n" $PAD "" "$DONE" $(( BOX_W - PAD - ${#DONE} )) ""
echo "  ╚$(printf '═%.0s' $(seq 1 $BOX_W))╝"
echo ""
echo "  You're all set — close the terminal and open Plexi."
echo ""
echo "  (Shell completions activate in your next terminal session.)"
echo ""
