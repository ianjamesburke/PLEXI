#!/usr/bin/env bash
# Usage: scripts/install.sh [--channel main|alpha|beta] [--tag vX.Y.Z] [--dry-run]
#        scripts/install.sh --from-source [channel]
# Derives channel from git branch (main→main, alpha→alpha, beta→beta).
# Falls back to .channel file, then "main". Must be run from the repo root.
set -euo pipefail

# The release path is self-contained: consumers need neither a package manager
# nor a Rust toolchain. The historical installer below remains available for
# contributors as an explicit --from-source escape hatch.
if [[ "${1:-}" != "--from-source" ]]; then
  REPO_SLUG="ianjamesburke/PLEXI"
  CHANNEL="main"
  TAG=""
  DRY_RUN=0
  INSTALL_ROOT="${PLEXI_INSTALL_DIR:-$HOME/.local/share/plexi}"
  BIN_DIR="${PLEXI_BIN_DIR:-$HOME/.local/bin}"
  RELEASE_BASE="${PLEXI_RELEASE_BASE_URL:-https://github.com/${REPO_SLUG}/releases/download}"

  usage() {
    cat <<'EOF'
Usage: install.sh [--channel main|alpha|beta] [--tag vX.Y.Z] [--dry-run]

Installs a prebuilt Plexi release into a user-owned directory. No package
manager or compiler is required. --tag is mainly for updater/CI use.

Contributors with a checkout may use: scripts/install.sh --from-source [channel]
EOF
  }

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --channel|-c) CHANNEL="${2:?--channel requires a value}"; shift 2 ;;
      --channel=*|-c=*) CHANNEL="${1#*=}"; shift ;;
      --tag) TAG="${2:?--tag requires a value}"; shift 2 ;;
      --tag=*) TAG="${1#*=}"; shift ;;
      --dry-run) DRY_RUN=1; shift ;;
      --help|-h) usage; exit 0 ;;
      *) echo "error: unknown argument '$1'" >&2; usage >&2; exit 2 ;;
    esac
  done
  case "$CHANNEL" in main|alpha|beta) ;; *) echo "error: channel must be main, alpha, or beta" >&2; exit 2;; esac

  case "$(uname -s)" in
    Darwin) OS="macos" ;;
    Linux) OS="linux" ;;
    MINGW*|MSYS*|CYGWIN*)
      echo "error: curl | bash is not a Windows installer. Run this in Windows PowerShell instead:" >&2
      echo "  irm https://raw.githubusercontent.com/ianjamesburke/PLEXI/alpha/scripts/install-windows.ps1 | iex" >&2
      exit 1
      ;;
    *) echo "error: unsupported operating system: $(uname -s)" >&2; exit 1 ;;
  esac
  case "$(uname -m)" in
    x86_64|amd64) ARCH="x64" ;;
    arm64|aarch64) ARCH="arm64" ;;
    *) echo "error: unsupported architecture: $(uname -m)" >&2; exit 1 ;;
  esac
  ASSET="plexi-${OS}-${ARCH}"
  ASSET+=".tar.gz"

  # Stable releases have no suffix; alpha/beta use their newest prerelease that
  # actually publishes this platform asset (skip empty cuts waiting on Actions).
  if [[ -z "$TAG" ]]; then
    command -v curl >/dev/null 2>&1 || { echo "error: curl is required to download Plexi" >&2; exit 1; }
    candidates=""
    if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
      if [[ "$CHANNEL" == main ]]; then
        candidates="$(gh api "repos/${REPO_SLUG}/releases" --paginate --jq '.[].tag_name | select(test("^v[0-9]+\\.[0-9]+\\.[0-9]+$"))' 2>/dev/null || true)"
      else
        candidates="$(gh api "repos/${REPO_SLUG}/releases" --paginate --jq '.[].tag_name | select(test("^v[0-9]+\\.[0-9]+\\.[0-9]+-'"$CHANNEL"'\\.[0-9]+$"))' 2>/dev/null || true)"
      fi
    fi
    if [[ -z "$candidates" ]]; then
      releases="$(curl -fsSL -A 'plexi-installer' "https://api.github.com/repos/${REPO_SLUG}/releases")" || {
        echo "error: could not query Plexi releases (pass --tag, or authenticate gh)" >&2
        exit 1
      }
      if command -v jq >/dev/null 2>&1; then
        if [[ "$CHANNEL" == main ]]; then
          candidates="$(printf '%s' "$releases" | jq -r '.[].tag_name | select(test("^v[0-9]+\\.[0-9]+\\.[0-9]+$"))')"
        else
          candidates="$(printf '%s' "$releases" | jq -r --arg c "$CHANNEL" '.[].tag_name | select(test("^v[0-9]+\\.[0-9]+\\.[0-9]+-" + $c + "\\.[0-9]+$"))')"
        fi
      else
        if [[ "$CHANNEL" == main ]]; then
          candidates="$(printf '%s' "$releases" | grep -oE '"tag_name"[[:space:]]*:[[:space:]]*"v[0-9]+\.[0-9]+\.[0-9]+"' | grep -oE 'v[0-9]+\.[0-9]+\.[0-9]+')"
        else
          candidates="$(printf '%s' "$releases" | grep -oE '"tag_name"[[:space:]]*:[[:space:]]*"v[0-9]+\.[0-9]+\.[0-9]+-'"$CHANNEL"'\.[0-9]+"' | grep -oE 'v[0-9]+\.[0-9]+\.[0-9]+-'"$CHANNEL"'\.[0-9]+')"
        fi
      fi
    fi
    TAG=""
    while IFS= read -r candidate; do
      [[ -n "$candidate" ]] || continue
      code="$(curl -sI -o /dev/null -w '%{http_code}' -L "${RELEASE_BASE}/${candidate}/${ASSET}" || true)"
      if [[ "$code" == "200" ]]; then
        TAG="$candidate"
        break
      fi
    done <<< "$candidates"
    [[ -n "$TAG" ]] || {
      echo "error: no published ${CHANNEL} release with ${ASSET} found" >&2
      exit 1
    }
  fi

  URL="${RELEASE_BASE}/${TAG}/${ASSET}"
  CHECKSUM_URL="${URL}.sha256"
  binary_name="plexi"
  [[ "$CHANNEL" != main ]] && binary_name+="-${CHANNEL}"
  destination="${INSTALL_ROOT}/${CHANNEL}"

  echo "Plexi ${TAG} (${CHANNEL}, ${OS}/${ARCH})"
  echo "Asset: ${URL}"
  echo "Install: ${destination}; command: ${BIN_DIR}/${binary_name}"
  if [[ "$DRY_RUN" == 1 ]]; then exit 0; fi

  sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
      sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
      shasum -a 256 "$1" | awk '{print $1}'
    else
      echo "error: sha256sum or shasum is required to verify Plexi downloads" >&2
      return 1
    fi
  }

  tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
  archive="${tmp}/${ASSET}"
  checksum_file="${tmp}/${ASSET}.sha256"
  if ! curl -fL --retry 3 --connect-timeout 15 -o "$archive" "$URL"; then
    echo "error: could not download ${ASSET} for ${OS}/${ARCH} from ${TAG}; that release may not publish this platform asset" >&2
    exit 1
  fi
  if ! curl -fL --retry 3 --connect-timeout 15 -o "$checksum_file" "$CHECKSUM_URL"; then
    echo "error: could not download the SHA-256 checksum for ${ASSET}; refusing to install" >&2
    exit 1
  fi
  expected_checksum="$(awk -v asset="$ASSET" '$2 == asset || $2 == "*" asset { print $1; exit }' "$checksum_file")"
  if [[ ! "$expected_checksum" =~ ^[[:xdigit:]]{64}$ ]]; then
    echo "error: release checksum file did not contain a SHA-256 for ${ASSET}" >&2
    exit 1
  fi
  actual_checksum="$(sha256_file "$archive")"
  if [[ "$actual_checksum" != "$expected_checksum" ]]; then
    echo "error: SHA-256 mismatch for ${ASSET}; refusing to install" >&2
    exit 1
  fi

  mkdir -p "$BIN_DIR" "$tmp/unpack" "$tmp/staged"
  if ! tar -xzf "$archive" -C "$tmp/unpack"; then
    echo "error: downloaded ${ASSET} is not a valid release archive; refusing to install" >&2
    exit 1
  fi
  source_binary="$(find "$tmp/unpack" -name plexi -type f -print -quit)"
  [[ -n "$source_binary" ]] || { echo "error: release asset did not contain the Plexi binary" >&2; exit 1; }

  # Stage every mutable artifact before touching the live install. The commit
  # below can then either complete as a unit or restore the previous payload.
  staged_destination="$tmp/staged/payload"
  mkdir -p "$staged_destination"
  cp -R "$tmp/unpack/." "$staged_destination/"
  staged_binary="$tmp/staged/$binary_name"
  install -m 0755 "$source_binary" "$staged_binary"
  installed_binary="$BIN_DIR/$binary_name"
  profile_suffix=""
  [[ "$CHANNEL" != main ]] && profile_suffix="-$CHANNEL"
  profile_dir="${HOME}/.plexi${profile_suffix}"
  mkdir -p "$profile_dir"

  previous_destination="$tmp/previous-payload"
  previous_binary="$tmp/previous-binary"
  previous_tag="$tmp/previous-installed_tag"
  had_destination=0; had_binary=0; had_tag=0
  replacement_started=0; binary_replacement_started=0
  [[ -e "$destination" || -L "$destination" ]] && had_destination=1
  [[ -e "$installed_binary" || -L "$installed_binary" ]] && had_binary=1
  [[ -e "$profile_dir/installed_tag" ]] && { cp "$profile_dir/installed_tag" "$previous_tag"; had_tag=1; }
  rollback_install() {
    [[ "$replacement_started" == 1 ]] && rm -rf "$destination"
    [[ "$had_destination" == 1 && ( -e "$previous_destination" || -L "$previous_destination" ) ]] && mv "$previous_destination" "$destination"
    [[ "$binary_replacement_started" == 1 ]] && rm -f "$installed_binary"
    [[ "$had_binary" == 1 && ( -e "$previous_binary" || -L "$previous_binary" ) ]] && mv "$previous_binary" "$installed_binary"
    if [[ "$had_tag" == 1 ]]; then
      cp "$previous_tag" "$profile_dir/installed_tag"
    else
      rm -f "$profile_dir/installed_tag"
    fi
  }
  if [[ "$had_destination" == 1 ]] && ! mv "$destination" "$previous_destination"; then
    echo "error: could not prepare the existing install for replacement" >&2; exit 1
  fi
  replacement_started=1
  if ! mv "$staged_destination" "$destination"; then
    echo "error: could not activate the staged install; restoring the previous install" >&2
    rollback_install; exit 1
  fi
  if [[ "$had_binary" == 1 ]] && ! mv "$installed_binary" "$previous_binary"; then
    echo "error: could not prepare the existing command for replacement; restoring the previous install" >&2
    rollback_install; exit 1
  fi
  binary_replacement_started=1
  if ! mv "$staged_binary" "$installed_binary"; then
    echo "error: could not activate the new command; restoring the previous install" >&2
    rollback_install; exit 1
  fi
  if ! printf '%s\n' "$TAG" > "$profile_dir/installed_tag.new" || ! mv "$profile_dir/installed_tag.new" "$profile_dir/installed_tag"; then
    echo "error: could not record the installed release; restoring the previous install" >&2
    rollback_install; exit 1
  fi
  echo "Installed ${BIN_DIR}/${binary_name}."
  exit 0
fi
shift

os="$(uname)"
case "$os" in
  Darwin|Linux) ;;
  *)
    echo "install supports macOS and Linux only (this is $os)."
    exit 1
    ;;
esac

# rsync is standard on macOS but not on a minimal Debian/Fedora install, and
# every use here is "replace this directory with that one". Fall back to cp so
# the installer does not hard-require a package the user may not have.
sync_tree() {
  local src="$1" dest="$2"   # both are directories; src contents land in dest
  if command -v rsync &>/dev/null; then
    rsync -a "${src%/}/" "$dest"
  else
    mkdir -p "$dest"
    cp -R "${src%/}/." "$dest"
  fi
}

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
# Resolve target-dir from cargo metadata rather than hardcoding a path. The
# [build] target-dir in .cargo/config.toml is relative ("target"), so each
# worktree builds into its own target/ — worktrees do NOT share a build cache
# (cross-worktree reuse comes from sccache, not a shared target dir).
# Resolving via cargo metadata keeps this script correct regardless of where
# target-dir points; a hardcoded path would silently skip the binary because
# the bundle lands in one place while this script looks in another.
_target_dir="$(cargo metadata --format-version=1 --no-deps 2>/dev/null | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])' 2>/dev/null || echo "")"

# Preflight: fail fast if target-dir resolution failed rather than building and
# then discovering the bundle in the wrong place.
if [[ -z "$_target_dir" ]]; then
  echo "Error: could not resolve cargo target directory."
  echo "  Make sure 'cargo metadata' runs cleanly from $(pwd) and Python 3 is available."
  exit 1
fi

if [[ "$os" == "Darwin" ]]; then
  app_src="${_target_dir}/release/bundle/osx/Plexi.app"
  app_dest="/Applications/${display}.app"
  # The real binary inside the installed bundle. Every later reference — shim
  # target, symlink target, completions, pack seeding — goes through this one
  # name, so the Linux branch only has to redefine where it points.
  stable_bin="$app_dest/Contents/MacOS/plexi${suffix}"
  bin_dest="/usr/local/bin/plexi${suffix}"
else
  # Linux has no app bundle: the release binary is installed into a
  # per-channel dir under XDG data home and $bin_dest is the PATH entry that
  # points at it — the same two-level shape as macOS, so channel routing
  # below is identical apart from the target path. ~/.local/bin keeps the
  # whole install sudo-free.
  app_src="${_target_dir}/release/plexi"
  app_dest="${XDG_DATA_HOME:-$HOME/.local/share}/plexi${suffix}"
  stable_bin="$app_dest/bin/plexi${suffix}"
  bin_dest="${PLEXI_BIN_DIR:-$HOME/.local/bin}/plexi${suffix}"
fi
bin_dir="$(dirname "$bin_dest")"
profile_dir="$HOME/.plexi${suffix}"

bin_install_needs_sudo=false
if [[ ! -d "$bin_dir" || ! -w "$bin_dir" ]]; then
  # A missing directory under $HOME is ours to create; only a system-owned
  # prefix needs an admin prompt.
  if [[ "$bin_dir" == "$HOME"/* ]]; then
    mkdir -p "$bin_dir"
  else
    bin_install_needs_sudo=true
  fi
fi

# Non-interactive callers (background updater, CI) set PLEXI_SKIP_BIN_INSTALL=1
# to skip the sudo credential check and /usr/local/bin write. The .app bundle
# update is sufficient — the existing shim delegates to /Applications/Plexi.app.
skip_bin_install="${PLEXI_SKIP_BIN_INSTALL:-0}"

if [[ "$skip_bin_install" != "1" ]] && $bin_install_needs_sudo; then
  echo "CLI install needs admin access for $bin_dir"
  sudo -v
fi

# cargo-bundle only produces a macOS .app; on Linux the plain release binary
# is the artifact.
if [[ "$os" == "Darwin" ]]; then
  _build_cmd=(cargo bundle --release)
else
  _build_cmd=(cargo build --release)
fi

# Embed a second, compile-time identity for test-only behavior. The runtime
# basename remains necessary for profile routing, but is not a security
# boundary: any installed binary can be renamed. Explicitly clear the marker
# for every real channel so an inherited shell variable cannot taint a
# production build.
if [[ "$channel" =~ ^pr-[0-9]+$ ]]; then
  PLEXI_BUILD_TEST_CHANNEL="$channel" bash scripts/cargo-with-lease.sh "${_build_cmd[@]}"
else
  env -u PLEXI_BUILD_TEST_CHANNEL bash scripts/cargo-with-lease.sh "${_build_cmd[@]}"
fi

if [[ "$os" == "Darwin" ]]; then
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

  # Sign the assembled bundle with the stable "Plexi Dev" identity so its code
  # signature's designated requirement pins to the cert instead of a per-build
  # cdhash. macOS keys keychain "Always Allow" ACLs off the designated
  # requirement, so an ad-hoc bundle (fresh cdhash every rebuild) makes the AI
  # broker's OPENROUTER_API_KEY read re-prompt after every install; a stable
  # identity stops it. Must run AFTER every bundle mutation above (Info.plist
  # patch, binary rename) — any change invalidates the signature.
  # `just codesign-setup` creates the identity one time.
  if security find-identity -v -p codesigning 2>/dev/null | grep -q "Plexi Dev"; then
    if codesign --force --deep --sign "Plexi Dev" "$app_dest" && codesign --verify "$app_dest"; then
      echo "Signed $app_dest with stable 'Plexi Dev' identity (no per-install keychain re-prompt)"
    else
      echo "warning: codesign with 'Plexi Dev' failed — installed UNSIGNED; the AI-broker keychain re-prompt will persist"
    fi
  else
    echo "warning: no 'Plexi Dev' code-signing identity — installing UNSIGNED; run 'just codesign-setup' once to stop the per-install AI-broker keychain re-prompt"
  fi
else
  if [[ ! -x "$app_src" ]]; then
    echo "Error: release binary not found at: $app_src"
    echo "  Expected 'cargo build --release' to produce it there."
    exit 1
  fi

  # Install the binary under its channel name: config_dir_name() resolves the
  # profile from current_exe()'s basename, so a binary installed as plain
  # "plexi" would read ~/.plexi/ no matter which channel built it.
  mkdir -p "$app_dest/bin"
  install -m 0755 "$app_src" "$stable_bin"

  # A .desktop entry is what makes this a real Linux install rather than a
  # loose binary: the launcher, the dock and xdg-open all read it.
  desktop_dir="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
  icon_dir="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/512x512/apps"
  mkdir -p "$desktop_dir" "$icon_dir"
  icon_name="plexi${suffix}"
  if [[ -f "$REPO_ROOT/assets/app-icon.png" ]]; then
    cp "$REPO_ROOT/assets/app-icon.png" "$icon_dir/${icon_name}.png"
  fi
  cat > "$desktop_dir/${icon_name}.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=$display
Comment=Spatial terminal window manager
Exec=$stable_bin
Icon=$icon_name
Terminal=false
Categories=Development;System;TerminalEmulator;
StartupWMClass=plexi
EOF
  chmod 0644 "$desktop_dir/${icon_name}.desktop"
  if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "$desktop_dir" 2>/dev/null || true
  fi
  echo "Desktop entry: $desktop_dir/${icon_name}.desktop"
fi

if [[ "$skip_bin_install" != "1" ]]; then
  if [[ "$channel" == "main" ]]; then
    # Main owns the bare `plexi` PATH command, but installs it as a contextual
    # shim instead of a symlink. Inside a Plexi PTY, the shim delegates to the
    # active channel binary from PLEXI_CHANNEL. Outside Plexi, it runs stable.
    # Remove first so an old symlink is not followed when writing the script.
    shim_tmp="$(mktemp)"
    # The two install-time paths are interpolated; the logic below is literal.
    cat > "$shim_tmp" <<EOF
#!/usr/bin/env bash
set -euo pipefail

stable_binary="$stable_bin"
channel_bin_dir="$bin_dir"
EOF
    cat >> "$shim_tmp" <<'EOF'

if [[ ! -x "$stable_binary" ]]; then
  echo "error: stable Plexi binary not found at $stable_binary" >&2
  exit 1
fi

if [[ -n "${PLEXI_CHANNEL:-}" ]]; then
  channel_binary="$channel_bin_dir/plexi-${PLEXI_CHANNEL}"
  if [[ -x "$channel_binary" ]]; then
    exec "$channel_binary" "$@"
  fi
fi

exec "$stable_binary" "$@"
EOF
    if $bin_install_needs_sudo; then
      [[ -d "$bin_dir" ]] || sudo mkdir -p "$bin_dir"
      sudo rm -f "$bin_dest"
      sudo install -m 0755 "$shim_tmp" "$bin_dest"
    else
      [[ -d "$bin_dir" ]] || mkdir -p "$bin_dir"
      rm -f "$bin_dest"
      install -m 0755 "$shim_tmp" "$bin_dest"
    fi
    rm -f "$shim_tmp"
  else
    if $bin_install_needs_sudo; then
      [[ -d "$bin_dir" ]] || sudo mkdir -p "$bin_dir"
      sudo ln -sf "$stable_bin" "$bin_dest"
    else
      [[ -d "$bin_dir" ]] || mkdir -p "$bin_dir"
      ln -sf "$stable_bin" "$bin_dest"
    fi
  fi
else
  echo "Skipping bin install (PLEXI_SKIP_BIN_INSTALL=1 — shim unchanged at $bin_dest)"
fi

# Install shell completions for production and release-candidate channels.
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

if [[ "$skip_bin_install" != "1" ]] && [[ "$channel" == "main" || "$channel" == "alpha" || "$channel" == "beta" || "$channel" == rc-* ]]; then
  if [[ "$channel" == "main" ]]; then
    install_completions "$stable_bin"
  else
    install_completions "$bin_dest"
  fi
fi

mkdir -p "$profile_dir/sdk" "$profile_dir/apps" "$profile_dir/agents" "$profile_dir/scripts"

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
cp sdk/python/pyproject.toml "$profile_dir/sdk/pyproject.toml"
rm -rf "$profile_dir/sdk/plexi_sdk.old" "$profile_dir/sdk/plexi_sdk.py"
find "$profile_dir/sdk/plexi_sdk" -name '__pycache__' -type d -exec rm -rf {} + 2>/dev/null || true
# App-seeding policy is channel-gated (see scripts/AGENTS.md). packs/core.toml
# is the single source of truth for the maintained/core app set.
#   alpha / pr-*  → sync maintained top-level app dirs immediately so branch
#                   app changes are visible in PR installs.
#   beta / main   → seed exactly the canonical set through the host's own pack
#                   applier so no app list is duplicated in this script.
#                   --refresh re-extracts already-installed core apps from the
#                   new binary's embedded tree — this is how core-app updates
#                   reach existing stable profiles (launch reseed only installs
#                   missing apps). Example/demo apps seed only for a fresh
#                   profile (apps_was_empty), matching the host's first-launch
#                   behavior. POC apps under dev/ and non-pack demos never
#                   reach channel app registries.
if [[ "$channel" == alpha || "$channel" =~ ^pr- ]]; then
  mkdir -p "$profile_dir/apps"
  # apps/wasm-poc/* is excluded: raw cargo-component WASM POCs with no build
  # step at install time — syncing them hijacks app-name slots (e.g. "counter")
  # in every fresh profile with an unbuilt/unbuildable app (stint 0428). They
  # are not Plexi apps (see apps/AGENTS.md) and are never in packs/core.toml.
  # maxdepth stays 3 (not 2) in case any future maintained app nests one level
  # deeper than top-level apps/<name>/manifest.toml; the registry scan is flat
  # (one dir per app), so app_name below resolves correctly at any depth.
  find apps -mindepth 2 -maxdepth 3 -name manifest.toml -not -path 'apps/dev/*' -not -path 'apps/examples/*' -not -path 'apps/wasm-poc/*' | while read -r manifest; do
    app_dir="$(dirname "$manifest")"
    app_name="$(basename "$app_dir")"
    rm -rf "$profile_dir/apps/$app_name"
    sync_tree "$app_dir" "$profile_dir/apps/$app_name"
    # Flattening changes app_dir's depth relative to the source tree
    # (nested app dir → apps/<name>), so a manifest `entry` that escapes
    # app_dir with `../` no longer resolves post-flatten even though it
    # resolves correctly for path-based open, which reads the manifest
    # in place. Bundle the compiled artifact alongside the synced
    # manifest and rewrite its entry to the bundled basename so id-based
    # open is self-contained, matching every other installed app. This
    # is generic entry-rewriting logic, not wasm-poc-specific — it is
    # simply unreached for wasm-poc today since that tree is excluded
    # above (stint 0428); it still applies to any other nested app with
    # a `../`-relative entry.
    python3 - "$app_dir" "$profile_dir/apps/$app_name" <<'PYEOF'
import os
import shutil
import sys
import tomllib

app_dir, dest_dir = sys.argv[1], sys.argv[2]
with open(os.path.join(app_dir, "manifest.toml"), "rb") as f:
    entry = tomllib.load(f)["app"]["entry"]

if not entry.startswith(".."):
    sys.exit(0)  # same-dir entry already resolves correctly post-flatten

src = os.path.normpath(os.path.join(app_dir, entry))
if not os.path.isfile(src):
    sys.exit(0)  # not built yet; resolve_entry's "build it first" error still applies

basename = os.path.basename(entry)
shutil.copy2(src, os.path.join(dest_dir, basename))

dest_manifest = os.path.join(dest_dir, "manifest.toml")
with open(dest_manifest) as f:
    text = f.read()
old_line = f'entry = "{entry}"'
new_line = f'entry = "{basename}"'
if old_line not in text:
    sys.exit(f"error: could not find '{old_line}' in {dest_manifest} to rewrite")
with open(dest_manifest, "w") as f:
    f.write(text.replace(old_line, new_line, 1))
PYEOF
  done
else
  bundled_bin="$stable_bin"
  # Unset any inherited PLEXI_CHANNEL (in-pane installs leak it) so the
  # binary resolves its profile from its own basename — PLEXI_CHANNEL wins
  # over the basename and would seed the wrong profile ("main" has no valid
  # PLEXI_CHANNEL value, so setting it explicitly is not an option).
  env -u PLEXI_CHANNEL "$bundled_bin" app install --pack core --refresh >/dev/null 2>&1 \
    || echo "warning: core pack refresh failed — launch only installs missing apps; run 'plexi${suffix} app install --pack core --refresh' to update core apps"
  if $apps_was_empty; then
    env -u PLEXI_CHANNEL "$bundled_bin" app install --pack packs/examples.toml >/dev/null 2>&1 \
      || echo "note: examples pack seed deferred to first launch"
  fi
fi
find "$profile_dir/apps" -maxdepth 2 -name 'plexi_sdk.py' -delete 2>/dev/null || true
find "$profile_dir/apps" -name '*.py' -exec chmod +x {} \;

if [[ "$os" == "Darwin" ]]; then
  lsregister_bin="/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister"
  if [[ -x "$lsregister_bin" ]]; then
    "$lsregister_bin" -f "$app_dest" 2>/dev/null || echo "note: lsregister -f failed"
  fi
  /System/Library/CoreServices/pbs -update 2>/dev/null || echo "note: pbs -update failed"
fi

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
"$SCRIPT_DIR/migrate-config.sh" "$CONFIG" "[notifications]" "[theme]" "[effects]"

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

# Install bundled agent definitions.
install_agents() {
  local agents_dest="$profile_dir/agents"
  local repo_agents="$REPO_ROOT/agents"
  [[ -d "$repo_agents" ]] || return 0
  local installed=0

  for agent_dir in "$repo_agents"/*/; do
    [[ -f "$agent_dir/settings.toml" ]] || continue
    local name
    name="$(basename "$agent_dir")"
    mkdir -p "$agents_dest/$name"
    sync_tree "$agent_dir" "$agents_dest/$name"
    installed=$((installed + 1))
  done

  if [[ $installed -gt 0 ]]; then
    echo "Agents: $installed installed to $agents_dest/"
  fi
}

install_agents

# Record the release tag so the updater knows the exact installed version.
# Priority: env var (set by source update) > git tag at HEAD.
if [[ -n "${PLEXI_INSTALL_TAG:-}" ]]; then
  echo "$PLEXI_INSTALL_TAG" > "$profile_dir/installed_tag"
  echo "Release tag: $PLEXI_INSTALL_TAG"
else
  _head_tag="$(git -C "$REPO_ROOT" describe --tags --exact-match HEAD 2>/dev/null || true)"
  if [[ -n "$_head_tag" ]]; then
    echo "$_head_tag" > "$profile_dir/installed_tag"
    echo "Release tag: $_head_tag (from git)"
  fi
fi

echo "Installed $app_dest"
echo "CLI: $bin_dest"
echo "Config dir: $profile_dir/"
if [[ "$channel" == alpha || "$channel" =~ ^pr- ]]; then
  echo "Apps: $(ls "$profile_dir/apps" | wc -l | tr -d ' ') synced from top-level app manifests"
else
  echo "Apps: $(ls "$profile_dir/apps" 2>/dev/null | wc -l | tr -d ' ') seeded from canonical packs (stable channel)"
fi
case ":${PATH}:" in
  *":${bin_dir}:"*) ;;
  *)
    echo ""
    echo "warning: $bin_dir is not on your PATH — '$(basename "$bin_dest")' will not be found."
    echo "  Add this to your shell profile:  export PATH=\"$bin_dir:\$PATH\""
    ;;
esac
if ! command -v micro &>/dev/null; then
  if [[ "$os" == "Darwin" ]]; then
    echo "tip: brew install micro — preferred editor for plexi notes open"
  else
    echo "tip: install micro (apt install micro / dnf install micro) — preferred editor for plexi notes open"
  fi
fi
echo ""
echo "New to shell configuration? https://github.com/ianjamesburke/dotfiles has a starter setup and explanation."
