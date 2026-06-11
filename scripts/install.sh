#!/usr/bin/env bash
# Usage: scripts/install.sh [channel]
# Derives channel from git branch (main→main, alpha→alpha, beta→beta).
# Falls back to .channel file, then "main". Must be run from the repo root.
set -euo pipefail

if [[ "$(uname)" != "Darwin" ]]; then
  echo "install is macOS-only."
  exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

_git_channel() {
  local branch
  branch="$(git -C "$REPO_ROOT" branch --show-current 2>/dev/null || echo "")"
  case "$branch" in
    main)   echo "main" ;;
    alpha)  echo "alpha" ;;
    beta)   echo "beta" ;;
    *)      cat .channel 2>/dev/null || echo "main" ;;
  esac
}

channel="${1:-$(_git_channel)}"

if [[ "$channel" == "main" ]]; then
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
# Resolve target-dir from cargo metadata so all worktrees share a single build
# cache. The CARGO_TARGET_DIR env var or a [build] target-dir in .cargo/config.toml
# points every worktree at <repo-root>/target/ rather than a per-worktree target/.
# INVARIANT: if you change the target-dir in .cargo/config.toml, update this
# resolver too. A mismatch causes install to silently skip the binary because the
# bundle lands in the old path while this script looks in the new one.
_target_dir="$(cargo metadata --format-version=1 --no-deps 2>/dev/null | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])' 2>/dev/null || echo "")"

# Preflight: fail fast if target-dir resolution failed rather than building and
# then discovering the bundle in the wrong place.
if [[ -z "$_target_dir" ]]; then
  echo "Error: could not resolve cargo target directory."
  echo "  Make sure 'cargo metadata' runs cleanly from $(pwd) and Python 3 is available."
  exit 1
fi

app_src="${_target_dir}/release/bundle/osx/Plexi.app"
app_dest="/Applications/${display}.app"
bin_dest="/usr/local/bin/plexi${suffix}"
profile_dir="$HOME/.plexi${suffix}"

cargo bundle --release

if [[ ! -d "$app_src" ]]; then
  echo "Error: bundle not found at: $app_src"
  echo "  Expected cargo to produce the bundle at that path."
  echo "  If you changed target-dir in .cargo/config.toml, update the cargo metadata"
  echo "  resolver in scripts/install.sh to match (see INVARIANT comment above)."
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

# For non-main channels, rename the binary inside the installed bundle from
# "plexi" to "plexi-<channel>" and update CFBundleExecutable to match.
# config_dir_name() detects the channel from current_exe() file_name(), so the
# binary name inside the bundle must contain the channel suffix or the app
# silently reads ~/.plexi/apps/ instead of ~/.plexi-<channel>/apps/.
if [[ -n "$suffix" ]]; then
  mv "$app_dest/Contents/MacOS/plexi" "$app_dest/Contents/MacOS/plexi${suffix}"
fi

ln -sf "$app_dest/Contents/MacOS/plexi${suffix}" "$bin_dest"

# Only the main channel owns the bare `plexi` symlink.
# PR builds and development channels are excluded.
if [[ "$channel" == "main" ]]; then
  ln -sf "$app_dest/Contents/MacOS/plexi${suffix}" /usr/local/bin/plexi
fi

# Install shell completions for production channels (main, alpha, beta).
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

if [[ "$channel" == "main" || "$channel" == "alpha" || "$channel" == "beta" ]]; then
  install_completions "$bin_dest"
fi

mkdir -p "$profile_dir/sdk" "$profile_dir/apps" "$profile_dir/scripts"

# Seed default scripts (skip files already present to preserve user customizations).
DEFAULT_SCRIPTS_DIR="$REPO_ROOT/scripts/default-scripts"
if [[ -d "$DEFAULT_SCRIPTS_DIR" ]]; then
  for script in "$DEFAULT_SCRIPTS_DIR"/*; do
    [[ -f "$script" ]] || continue
    name="$(basename "$script")"
    dest="$profile_dir/scripts/$name"
    if [[ ! -f "$dest" ]]; then
      cp "$script" "$dest"
      chmod +x "$dest"
    fi
  done
fi
apps_was_empty=true
if find "$profile_dir/apps" -mindepth 1 -maxdepth 1 -print -quit 2>/dev/null | grep -q .; then
  apps_was_empty=false
fi
rm -rf "$profile_dir/sdk/plexi_sdk.tmp" "$profile_dir/sdk/plexi_sdk.old"
cp -R sdk/python/plexi_sdk "$profile_dir/sdk/plexi_sdk.tmp"
mv "$profile_dir/sdk/plexi_sdk" "$profile_dir/sdk/plexi_sdk.old" 2>/dev/null || true
mv "$profile_dir/sdk/plexi_sdk.tmp" "$profile_dir/sdk/plexi_sdk"
rm -rf "$profile_dir/sdk/plexi_sdk.old" "$profile_dir/sdk/plexi_sdk.py"
find "$profile_dir/sdk/plexi_sdk" -name '__pycache__' -type d -exec rm -rf {} + 2>/dev/null || true
# Re-seed all production apps on every install; always sync dev apps on alpha/PR.
rsync -a --exclude=dev/ apps/ "$profile_dir/apps/"
if [[ "$channel" == alpha || "$channel" =~ ^pr- ]]; then
  rsync -a apps/dev/ "$profile_dir/apps/"
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
if [[ "$channel" == "alpha" ]]; then
  # Alpha always runs with the default template — never user-customized.
  # PR builds seed their config from alpha, so a custom alpha would
  # pollute every PR channel. Reset on every install.
  cat "$SCRIPT_DIR/default-config.toml" > "$CONFIG"
  echo "config: reset to defaults (alpha channel stays default)"
elif [ ! -f "$CONFIG" ]; then
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
if ! command -v micro &>/dev/null; then
  echo "tip: brew install micro — preferred editor for plexi notes open"
fi
echo ""
echo "New to shell configuration? https://github.com/ianjamesburke/dotfiles has a starter setup and explanation."
