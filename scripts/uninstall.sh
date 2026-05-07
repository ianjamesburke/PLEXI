#!/usr/bin/env bash
# Remove one or all installed Plexi channels.
# Usage: scripts/uninstall.sh [channel]
#   channel: stable | alpha | beta | pr-<N> | all (default: all)
#
# Profile dirs, app bundles, and CLI binaries are removed per channel.
# Shell integration and completions are only removed when channel=all or stable.
# If a backlog folder exists inside a profile dir, it is archived to
# ~/plexi-backlog-archive/ before the profile dir is deleted.
set -euo pipefail

if [[ "$(uname)" != "Darwin" ]]; then
  echo "uninstall is macOS-only."
  exit 1
fi

channel="${1:-all}"
ARCHIVE_BASE="$HOME/plexi-backlog-archive"
removed=0

ok()   { printf '\033[1;32m ✓\033[0m %s\n' "$*"; }
info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }

remove_file() {
  if [[ -e "$1" || -L "$1" ]]; then
    rm -f "$1"
    ok "Removed $1"
    removed=1
  fi
}

remove_dir() {
  if [[ -d "$1" ]]; then
    rm -rf "$1"
    ok "Removed $1"
    removed=1
  fi
}

# Remove one channel's app bundle, CLI binary, and profile dir.
# Archives backlog from the profile dir if present.
uninstall_channel() {
  local suffix="$1"   # e.g. "" | "-alpha" | "-beta" | "-pr-123"
  local cap="$2"      # e.g. "" | " Alpha" | " Beta" | " PR123"

  local profile_dir="$HOME/.plexi${suffix}"
  local app="/Applications/Plexi${cap}.app"
  local bin="/usr/local/bin/plexi${suffix}"

  # Archive backlog before deleting the profile dir
  local backlog_src="$profile_dir/backlog"
  if [[ -d "$backlog_src" ]]; then
    mkdir -p "$ARCHIVE_BASE"
    local archive_dest="$ARCHIVE_BASE/plexi${suffix}-backlog-$(date +%Y%m%d%H%M%S)"
    mv "$backlog_src" "$archive_dest"
    ok "Archived backlog → $archive_dest"
    removed=1
  fi

  remove_dir  "$profile_dir"
  remove_dir  "$app"
  remove_file "$bin"
}

# Remove shell integration snippet from rc files.
remove_shell_integration() {
  info "Shell integration"
  for rc in "$HOME/.zshrc" "$HOME/.bashrc"; do
    [[ -f "$rc" ]] || continue
    if grep -qF 'plexi shell-init' "$rc"; then
      perl -i -0pe 's/\n?# Plexi shell integration\neval "\$\(plexi shell-init\)"\n?//g' "$rc"
      ok "Removed shell integration from $rc"
      removed=1
    fi
  done
}

# Remove shell completions for all shells.
remove_completions() {
  info "Completions"
  if command -v brew &>/dev/null; then
    brew_zsh_dir="$(brew --prefix)/share/zsh/site-functions"
    remove_file "$brew_zsh_dir/_plexi"
  fi
  remove_file "$HOME/.zfunc/_plexi"
  remove_file "$HOME/.bash_completion.d/plexi"
  remove_file "$HOME/.config/fish/completions/plexi.fish"
}

# ── dispatch ──────────────────────────────────────────────────────────────────

case "$channel" in
  all)
    echo ""
    echo "This will remove all Plexi channels (stable, alpha, beta):"
    echo "  • /Applications/Plexi*.app"
    echo "  • /usr/local/bin/plexi*"
    echo "  • ~/.plexi*/  (profile directories)"
    echo "  • Shell integration from ~/.zshrc / ~/.bashrc"
    echo "  • Shell completions (zsh, bash, fish)"
    echo ""
    read -r -p "Proceed? [y/N] " confirm
    [[ "$confirm" =~ ^[Yy]$ ]] || { echo "Aborted."; exit 0; }
    echo ""

    info "Channel: stable"
    uninstall_channel "" ""
    info "Channel: alpha"
    uninstall_channel "-alpha" " Alpha"
    info "Channel: beta"
    uninstall_channel "-beta" " Beta"
    remove_shell_integration
    remove_completions
    ;;

  stable)
    echo ""
    echo "This will remove the stable Plexi channel:"
    echo "  • /Applications/Plexi.app"
    echo "  • /usr/local/bin/plexi"
    echo "  • ~/.plexi/  (profile directory)"
    echo "  • Shell integration from ~/.zshrc / ~/.bashrc"
    echo "  • Shell completions (zsh, bash, fish)"
    echo ""
    read -r -p "Proceed? [y/N] " confirm
    [[ "$confirm" =~ ^[Yy]$ ]] || { echo "Aborted."; exit 0; }
    echo ""

    info "Channel: stable"
    uninstall_channel "" ""
    remove_shell_integration
    remove_completions
    ;;

  alpha)
    info "Channel: alpha"
    uninstall_channel "-alpha" " Alpha"
    ;;

  beta)
    info "Channel: beta"
    uninstall_channel "-beta" " Beta"
    ;;

  pr-*)
    number="${channel#pr-}"
    [[ "$number" =~ ^[0-9]+$ ]] || { echo "error: invalid PR number '$number'"; exit 1; }
    info "Channel: PR $number"
    uninstall_channel "-pr-${number}" " PR${number}"
    ;;

  *)
    echo "error: unknown channel '$channel'"
    echo "Usage: just uninstall [stable|alpha|beta|pr-<N>|all]"
    exit 1
    ;;
esac

# ── done ──────────────────────────────────────────────────────────────────────

echo ""
if [[ $removed -eq 0 ]]; then
  echo "Nothing found to remove."
else
  echo "Done."
  if [[ -d "$ARCHIVE_BASE" ]]; then
    echo "Backlog archive: $ARCHIVE_BASE"
  fi
fi
