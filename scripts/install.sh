#!/usr/bin/env bash
# Usage: scripts/install.sh [channel]
# Derives channel from git branch (main→stable, alpha→alpha, beta→beta).
# Falls back to .channel file, then "stable". Must be run from the repo root.
set -euo pipefail

if [[ "$(uname)" != "Darwin" ]]; then
  echo "install is macOS-only."
  exit 1
fi

_git_channel() {
  local branch
  branch="$(git branch --show-current 2>/dev/null || echo "")"
  case "$branch" in
    main)   echo "stable" ;;
    alpha)  echo "alpha" ;;
    beta)   echo "beta" ;;
    *)      cat .channel 2>/dev/null || echo "stable" ;;
  esac
}

channel="${1:-$(_git_channel)}"

if [[ "$channel" == "stable" ]]; then
  cap=""
  suffix=""
elif [[ "$channel" =~ ^pr-([0-9]+)$ ]]; then
  cap=" PR${BASH_REMATCH[1]}"
  suffix="-$channel"
else
  cap=" $(echo "$channel" | awk '{print toupper(substr($0,1,1)) substr($0,2)}')"
  suffix="-$channel"
fi

display="Plexi${cap}"
bundle_id="com.ianjamesburke.plexi${suffix}"
# Resolve target-dir from .cargo/config.toml (worktrees share a single target/).
_target_dir="$(cargo metadata --format-version=1 --no-deps 2>/dev/null | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])' 2>/dev/null || echo "target")"
app_src="${_target_dir}/release/bundle/osx/Plexi.app"
app_dest="/Applications/${display}.app"
bin_dest="/usr/local/bin/plexi${suffix}"
profile_dir="$HOME/.plexi${suffix}"

cargo bundle --release

if [[ ! -d "$app_src" ]]; then
  echo "Error: bundle not found at $app_src"
  exit 1
fi

rm -rf "$app_dest"
cp -R "$app_src" "$app_dest"

# cargo-bundle reads bundle metadata from Cargo.toml and has no per-run
# override for app name or bundle ID. Keep the manifest canonical, then patch
# the copied bundle so installs never dirty tracked source files.
/usr/bin/plutil -replace CFBundleName -string "$display" "$app_dest/Contents/Info.plist"
/usr/bin/plutil -replace CFBundleDisplayName -string "$display" "$app_dest/Contents/Info.plist"
/usr/bin/plutil -replace CFBundleIdentifier -string "$bundle_id" "$app_dest/Contents/Info.plist"
/usr/bin/plutil -replace CFBundleExecutable -string "plexi${suffix}" "$app_dest/Contents/Info.plist"

# For non-stable channels, rename the binary inside the installed bundle from
# "plexi" to "plexi-<channel>" and update CFBundleExecutable to match.
# config_dir_name() detects the channel from current_exe() file_name(), so the
# binary name inside the bundle must contain the channel suffix or the app
# silently reads ~/.plexi/apps/ instead of ~/.plexi-<channel>/apps/.
if [[ -n "$suffix" ]]; then
  mv "$app_dest/Contents/MacOS/plexi" "$app_dest/Contents/MacOS/plexi${suffix}"
fi

ln -sf "$app_dest/Contents/MacOS/plexi${suffix}" "$bin_dest"

# For non-PR channels (alpha, beta, stable), also update the bare `plexi`
# symlink so that `plexi open`, `plexi notify`, etc. always reach the correct
# running instance regardless of which channel was most recently installed.
# PR builds are excluded — they are ephemeral and should not capture the bare name.
if [[ ! "$channel" =~ ^pr- ]]; then
  ln -sf "$app_dest/Contents/MacOS/plexi${suffix}" /usr/local/bin/plexi
fi

# Install shell completions (non-PR builds only).
# Called after the binary symlink is in place at $bin_dest.
install_completions() {
  local binary="$1"
  local binary_name
  binary_name="$(basename "$binary")"

  # zsh: prefer Homebrew site-functions (already on fpath), else ~/.zfunc/
  if command -v brew &>/dev/null; then
    local brew_zsh_dir
    brew_zsh_dir="$(brew --prefix)/share/zsh/site-functions"
    if [[ -d "$brew_zsh_dir" ]]; then
      "$binary" completions zsh > "$brew_zsh_dir/_${binary_name}"
      echo "Completions (zsh): $brew_zsh_dir/_${binary_name}"
    fi
  else
    mkdir -p "$HOME/.zfunc"
    "$binary" completions zsh > "$HOME/.zfunc/_${binary_name}"
    echo "Completions (zsh): ~/.zfunc/_${binary_name}"
    echo "  note: add 'fpath=(~/.zfunc \$fpath)' to ~/.zshrc if not already present"
  fi

  # bash: ~/.bash_completion.d/
  mkdir -p "$HOME/.bash_completion.d"
  "$binary" completions bash > "$HOME/.bash_completion.d/${binary_name}"
  echo "Completions (bash): ~/.bash_completion.d/${binary_name}"

  # fish: only if fish is installed
  if [[ -d "$HOME/.config/fish" ]]; then
    mkdir -p "$HOME/.config/fish/completions"
    "$binary" completions fish > "$HOME/.config/fish/completions/${binary_name}.fish"
    echo "Completions (fish): ~/.config/fish/completions/${binary_name}.fish"
  fi
}

if [[ ! "$channel" =~ ^pr- ]]; then
  install_completions "$bin_dest"
  # For non-stable channels, also install bare `plexi` completions so the bare
  # symlink (pointing to this channel's binary) gets its own completion entry.
  if [[ "$channel" != "stable" ]]; then
    install_completions /usr/local/bin/plexi
  fi
fi

mkdir -p "$profile_dir/sdk" "$profile_dir/apps" "$profile_dir/scripts"
rm -rf "$profile_dir/sdk/plexi_sdk.tmp" "$profile_dir/sdk/plexi_sdk.old"
cp -R sdk/python/plexi_sdk "$profile_dir/sdk/plexi_sdk.tmp"
mv "$profile_dir/sdk/plexi_sdk" "$profile_dir/sdk/plexi_sdk.old" 2>/dev/null || true
mv "$profile_dir/sdk/plexi_sdk.tmp" "$profile_dir/sdk/plexi_sdk"
rm -rf "$profile_dir/sdk/plexi_sdk.old" "$profile_dir/sdk/plexi_sdk.py"
find "$profile_dir/sdk/plexi_sdk" -name '__pycache__' -type d -exec rm -rf {} + 2>/dev/null || true
# Seed core and example apps; alpha/PR builds also get dev-examples.
rsync -a apps/core/ "$profile_dir/apps/"
rsync -a apps/examples/ "$profile_dir/apps/"
if [[ "$channel" == alpha || "$channel" =~ ^pr- ]]; then
  rsync -a dev-examples/ "$profile_dir/apps/"
fi
find "$profile_dir/apps" -maxdepth 2 -name 'plexi_sdk.py' -delete 2>/dev/null || true
find "$profile_dir/apps" -name '*.py' -exec chmod +x {} \;

lsregister_bin="/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister"
if [[ -x "$lsregister_bin" ]]; then
  "$lsregister_bin" -f "$app_dest" 2>/dev/null || echo "note: lsregister -f failed"
fi
/System/Library/CoreServices/pbs -update 2>/dev/null || echo "note: pbs -update failed"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

CONFIG="$profile_dir/config.toml"
if [ ! -f "$CONFIG" ]; then
  ALPHA_CONFIG="$HOME/.plexi-alpha/config.toml"
  if [[ "$channel" == pr-* ]] && [ -f "$ALPHA_CONFIG" ]; then
    cp "$ALPHA_CONFIG" "$CONFIG"
    echo "config: seeded from alpha config"
  else
    cat "$SCRIPT_DIR/default-config.toml" > "$CONFIG"
    echo "config: created default config at $CONFIG — set OPENROUTER_API_KEY in your shell profile"
  fi
fi

# Ensure required top-level config sections are present (additive-only migration).
# [ai] is intentionally omitted — it's commented out in the template (coming soon).
"$SCRIPT_DIR/migrate-config.sh" "$CONFIG" "[notifications]" "[theme]" "[beta]"

# Install agent skills for terminal AI assistants.
install_skills() {
  local skills_dest="$profile_dir/.agents/skills"
  local repo_skills="$SCRIPT_DIR/../skills"
  local installed=0

  for skill_dir in "$repo_skills"/*/; do
    [[ -f "$skill_dir/SKILL.md" ]] || continue
    local name
    name="$(basename "$skill_dir")"
    mkdir -p "$skills_dest/$name"
    cp -R "$skill_dir"/* "$skills_dest/$name/"
    installed=$((installed + 1))
  done

  if [[ $installed -gt 0 ]]; then
    echo "Skills: $installed installed to $skills_dest/"
    echo "  Agents can use these skills from $skills_dest/ (move them to your preferred location if needed)"
  fi
}

install_skills

echo "Installed $app_dest"
echo "CLI: $bin_dest"
echo "Config dir: $profile_dir/"
echo "Apps: $(ls "$profile_dir/apps" | wc -l | tr -d ' ') synced from apps/"
echo ""
echo "New to shell configuration? https://github.com/ianjamesburke/dotfiles has a starter setup and explanation."
