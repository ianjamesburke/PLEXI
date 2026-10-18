#!/usr/bin/env bash
# Cleanly uninstall one Plexi channel (or "all").
# Defaults to --delete-data (removes profile dir, leftover install dir, and
# Keychain items). Pass --keep-data (used by channel-clean.sh for standing
# channels) to retain user profile data and credentials.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
channel="all"
keep_data=0

for arg in "$@"; do
  case "$arg" in
    --keep-data) keep_data=1 ;;
    --delete-data) keep_data=0 ;;
    -*)
      echo "error: unknown flag '$arg' (expected --keep-data or --delete-data)" >&2
      exit 1
      ;;
    *) channel="$arg" ;;
  esac
done

profile_dir_for() {
  case "$1" in
    stable|main) echo "$HOME/.plexi" ;;
    *) echo "$HOME/.plexi-$1" ;;
  esac
}

norm_channel_for() {
  case "$1" in
    main) echo "stable" ;;
    *) echo "$1" ;;
  esac
}

purge_profile_keychain() {
  local profile="$1"
  [[ "$(uname -s)" == "Darwin" ]] || return 0
  command -v security >/dev/null 2>&1 || return 0
  local h hh
  h="$(printf '%s' "$profile" | shasum -a 256 | awk '{print $1}')"
  hh="$(printf '%s' "$profile/host" | shasum -a 256 | awk '{print $1}')"
  for acct in \
    "plexi:host:permission-seal:$h" \
    "plexi:host:permission-audit-tip:$h" \
    "plexi:host:needs-you-journal-tip:$hh"; do
    while security delete-generic-password -s "plexi-host-seal" -a "$acct" >/dev/null 2>&1; do :; done
  done
}

purge_all_plexi_keychain() {
  [[ "$(uname -s)" == "Darwin" ]] || return 0
  command -v security >/dev/null 2>&1 || return 0
  for svc in "plexi-host-seal" "plexi"; do
    while security delete-generic-password -s "$svc" >/dev/null 2>&1; do :; done
  done
}

archive_backlog_if_present() {
  local profile="$1"
  local ch="$2"
  if [[ -d "$profile/backlog" ]] && [[ -n "$(ls -A "$profile/backlog" 2>/dev/null || true)" ]]; then
    local archive_dir="$HOME/plexi-backlog-archive/${ch}-$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$archive_dir"
    cp -R "$profile/backlog/." "$archive_dir/"
    echo "Archived $profile/backlog to $archive_dir"
  fi
}

uninstall_one() {
  local raw_ch="$1"
  local norm_ch
  norm_ch="$(norm_channel_for "$raw_ch")"
  local profile
  profile="$(profile_dir_for "$norm_ch")"
  local data_flag="--delete-data"
  if (( keep_data )); then
    data_flag="--keep-data"
  else
    archive_backlog_if_present "$profile" "$norm_ch"
  fi

  local install_root="${XDG_DATA_HOME:-$HOME/.local/share}/plexi/$norm_ch"
  local reg_json="${XDG_DATA_HOME:-$HOME/.local/share}/plexi/installations/${norm_ch}.json"
  local bin_dir="${PLEXI_BIN_DIR:-$HOME/.local/bin}"
  local bin_name="plexi-$norm_ch"
  [[ "$norm_ch" != "stable" ]] || bin_name="plexi"

  local managed_status=0
  local binary=""
  if binary="$(bash "$script_dir/distribution-tool.sh" locate --channel "$norm_ch" 2>/dev/null)" && [[ -x "$binary" ]]; then
    env -u PLEXI_CHANNEL -u PLEXI_RUNNING -u PLEXI_SOCKET "$binary" uninstall --yes "$data_flag" || managed_status=$?
  else
    managed_status=2
  fi

  # Fallback: if the host binary was missing or its self-uninstall failed while
  # a receipt still exists on disk, invoke plexi-installer remove directly.
  if [[ "$managed_status" -ne 0 && -f "$install_root/installation.json" ]]; then
    if bash "$script_dir/distribution-tool.sh" remove --receipt "$install_root" >/dev/null 2>&1; then
      managed_status=0
    fi
  fi

  if (( ! keep_data )); then
    purge_profile_keychain "$profile"
    if [[ -d "$profile" ]]; then
      rm -rf "$profile"
      echo "Removed profile directory $profile."
    fi
    if [[ -d "$install_root" ]]; then
      rm -rf "$install_root"
      echo "Removed leftover install directory $install_root."
    fi
    if [[ -f "$reg_json" ]]; then
      rm -f "$reg_json"
    fi
    local repo_root
    repo_root="$(cd "$script_dir/.." && pwd)"
    if [[ "$norm_ch" != "stable" && -d "$repo_root/.plexi-$norm_ch" ]]; then
      rm -rf "$repo_root/.plexi-$norm_ch"
      echo "Removed workspace channel directory $repo_root/.plexi-$norm_ch."
    fi
    if [[ "$norm_ch" != "stable" && ( -e "$bin_dir/$bin_name" || -L "$bin_dir/$bin_name" ) ]]; then
      rm -f "$bin_dir/$bin_name"
      echo "Removed leftover launcher $bin_dir/$bin_name."
    fi
    if [[ "$(uname -s)" == "Darwin" && "$norm_ch" =~ ^pr-([0-9]+)$ ]]; then
      local pr_app="$HOME/Applications/Plexi PR${BASH_REMATCH[1]}.app"
      if [[ -L "$pr_app" || -d "$pr_app" ]]; then
        rm -rf "$pr_app"
        echo "Removed leftover app bundle $pr_app."
      fi
    fi
    return 0
  fi

  return "$managed_status"
}

if [[ "$channel" == "all" ]]; then
  channels=()
  if installed_json="$(bash "$script_dir/distribution-tool.sh" list 2>/dev/null)"; then
    while IFS= read -r ch; do
      [[ -n "$ch" ]] && channels+=("$ch")
    done < <(printf '%s' "$installed_json" | python3 -c 'import json,sys; print("\n".join(r.get("channel","") for r in json.load(sys.stdin)))' 2>/dev/null || true)
  fi
  for ch in stable alpha beta; do
    channels+=("$ch")
  done
  for dir in "${XDG_DATA_HOME:-$HOME/.local/share}"/plexi/*/; do
    [[ -d "$dir" ]] || continue
    base="$(basename "$dir")"
    [[ "$base" == "installations" || "$base" == "integration-lock" ]] && continue
    channels+=("$base")
  done
  if (( ! keep_data )); then
    for dir in "$HOME"/.plexi-*/; do
      [[ -d "$dir" ]] || continue
      base="$(basename "$dir" | sed 's/^\.plexi-//')"
      [[ "$base" == "src" ]] && continue
      channels+=("$base")
    done
  fi

  seen=" "
  for ch in "${channels[@]}"; do
    norm="$(norm_channel_for "$ch")"
    [[ "$seen" == *" $norm "* ]] && continue
    seen+="$norm "
    uninstall_one "$norm" || true
  done

  if (( ! keep_data )); then
    purge_all_plexi_keychain
    plexi_share="${XDG_DATA_HOME:-$HOME/.local/share}/plexi"
    if [[ -d "$plexi_share" ]]; then
      rm -rf "$plexi_share/integration-lock"
      rmdir "$plexi_share/installations" 2>/dev/null || true
      rmdir "$plexi_share" 2>/dev/null || true
    fi
    echo "Clean uninstall complete (all channels, profiles, and Keychain items removed)."
  fi
  exit 0
fi

uninstall_one "$channel"
