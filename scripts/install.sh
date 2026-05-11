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
app_src="target/release/bundle/osx/${display}.app"
app_dest="/Applications/${display}.app"
bin_dest="/usr/local/bin/plexi${suffix}"
profile_dir="$HOME/.plexi${suffix}"

# HACK: cargo-bundle reads bundle metadata from Cargo.toml directly with no
# env-var or CLI override. Patch the two bundle fields, build, then restore.
# Package `name` is left as "plexi" so Cargo.lock stays identical across branches.
if [[ -n "$suffix" ]]; then
  backup_dir="$(mktemp -d)"
  cp Cargo.toml "$backup_dir/Cargo.toml"
  cleanup() { cp "$backup_dir/Cargo.toml" Cargo.toml; rm -rf "$backup_dir"; }
  trap cleanup EXIT
  sed -i '' "s/name = \"Plexi[^\"]*\"/name = \"${display}\"/" Cargo.toml
  sed -i '' "s/identifier = \"com.ianjamesburke.plexi[^\"]*\"/identifier = \"${bundle_id}\"/" Cargo.toml
fi

cargo bundle --release

if [[ ! -d "$app_src" ]]; then
  echo "Error: bundle not found at $app_src"
  exit 1
fi

rm -rf "$app_dest"
cp -R "$app_src" "$app_dest"

# For non-stable channels, rename the binary inside the installed bundle from
# "plexi" to "plexi-<channel>" and update CFBundleExecutable to match.
# config_dir_name() detects the channel from current_exe() file_name(), so the
# binary name inside the bundle must contain the channel suffix or the app
# silently reads ~/.plexi/apps/ instead of ~/.plexi-<channel>/apps/.
if [[ -n "$suffix" ]]; then
  mv "$app_dest/Contents/MacOS/plexi" "$app_dest/Contents/MacOS/plexi${suffix}"
  /usr/bin/plutil -replace CFBundleExecutable -string "plexi${suffix}" "$app_dest/Contents/Info.plist"
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

  # zsh: prefer Homebrew site-functions (already on fpath), else ~/.zfunc/
  if command -v brew &>/dev/null; then
    local brew_zsh_dir
    brew_zsh_dir="$(brew --prefix)/share/zsh/site-functions"
    if [[ -d "$brew_zsh_dir" ]]; then
      "$binary" completions zsh > "$brew_zsh_dir/_plexi"
      echo "Completions (zsh): $brew_zsh_dir/_plexi"
    fi
  else
    mkdir -p "$HOME/.zfunc"
    "$binary" completions zsh > "$HOME/.zfunc/_plexi"
    echo "Completions (zsh): ~/.zfunc/_plexi"
    echo "  note: add 'fpath=(~/.zfunc \$fpath)' to ~/.zshrc if not already present"
  fi

  # bash: ~/.bash_completion.d/
  mkdir -p "$HOME/.bash_completion.d"
  "$binary" completions bash > "$HOME/.bash_completion.d/plexi"
  echo "Completions (bash): ~/.bash_completion.d/plexi"

  # fish: only if fish is installed
  if [[ -d "$HOME/.config/fish" ]]; then
    mkdir -p "$HOME/.config/fish/completions"
    "$binary" completions fish > "$HOME/.config/fish/completions/plexi.fish"
    echo "Completions (fish): ~/.config/fish/completions/plexi.fish"
  fi
}

if [[ ! "$channel" =~ ^pr- ]]; then
  install_completions "$bin_dest"
fi

mkdir -p "$profile_dir/sdk" "$profile_dir/apps"
rm -rf "$profile_dir/sdk/plexi_sdk.py" "$profile_dir/sdk/plexi_sdk"
cp -R sdk/python/plexi_sdk "$profile_dir/sdk/plexi_sdk"
find "$profile_dir/sdk/plexi_sdk" -name '__pycache__' -type d -exec rm -rf {} + 2>/dev/null || true
rsync -a --delete examples/ "$profile_dir/apps/"
find "$profile_dir/apps" -maxdepth 2 -name 'plexi_sdk.py' -delete 2>/dev/null || true
find "$profile_dir/apps" -name '*.py' -exec chmod +x {} \;

lsregister_bin="/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister"
if [[ -x "$lsregister_bin" ]]; then
  "$lsregister_bin" -f "$app_dest" 2>/dev/null || echo "note: lsregister -f failed"
fi
/System/Library/CoreServices/pbs -update 2>/dev/null || echo "note: pbs -update failed"

CONFIG="$profile_dir/config.toml"
if [ ! -f "$CONFIG" ]; then
  # Full config template — synced with CONFIG_TEMPLATE in src/config.rs.
  # To regenerate: python3 -c "import base64,sys; sys.stdout.write(base64.b64encode(open('scripts/default-config.toml','rb').read()).decode())"
  # (Once scripts/default-config.toml exists as the single source of truth — see issue #1118)
  echo "IyDilZTilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZcKIyDilZEgIFBsZXhpIENvbmZpZ3VyYXRpb24gICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAg4pWRCiMg4pWRICBDaGFuZ2VzIHRha2UgZWZmZWN0IG9uIG5leHQgbGF1bmNoLiAgICAgICAgICAgICAgICAgICAgICAgIOKVkQojIOKVmuKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVnQoKZm9udF9zaXplID0gMTQuMAoKIyDilIDilIAgVGhlbWUg4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSACiMgUGljayBhIHByZXNldCBPUiBjdXN0b21pemUgaW5kaXZpZHVhbCBjb2xvcnMgYmVsb3cuCiMgUHJlc2V0czogY2F0cHB1Y2Npbi1tb2NoYSwgZHJhY3VsYSwgdG9reW8tbmlnaHQsIGdydXZib3gtZGFyaywgbm9yZCwgc29sYXJpemVkLWRhcmsKdGhlbWVfcHJlc2V0ID0gImNhdHBwdWNjaW4tbW9jaGEiCgojIOKUgOKUgCBDb25maXJtYXRpb24gRGlhbG9ncyDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIAKIyBjb25maXJtX3F1aXQgcmVxdWlyZXMgdHJpcGxlIENtZCtRIHRvIGV4aXQgKHNhZmVyIHRoYW4gYSBzaW5nbGUgcHJlc3MpLgojIGNvbmZpcm1fY2xvc2Ugc2hvd3MgYSBkaWFsb2cgYmVmb3JlIENtZCtXIGNsb3NlcyBhIHBhbmUuCmNvbmZpcm1fcXVpdCAgPSB0cnVlCmNvbmZpcm1fY2xvc2UgPSBmYWxzZQoKIyDilIDilIAgTm90aWZpY2F0aW9ucyDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIAKIyBUaGUgd29yay1hcmVhIG1vZGFsIGlzIHRoZSBvbmUgYW5kIG9ubHkgbm90aWZpY2F0aW9uIHN1cmZhY2UuCiMgQXBwcyBlbWl0IGBjdHgubm90aWZ5KC4uLilgLCBgY3R4Lm5vdGlmeV9jaG9pY2UoLi4uKWAsIG9yCiMgYGN0eC5ub3RpZnlfaW5wdXQoLi4uKWAgYW5kIHRoZSBtb2RhbCByZW5kZXJzIGVhY2gga2luZCB3aXRoCiMga2V5Ym9hcmQtZmlyc3QgbmF2aWdhdGlvbiAoRW50ZXIgY29uZmlybXMsIGovayBvciDihpHihpMgY3ljbGUKIyBvcHRpb25zLCAxLTkgZGlyZWN0LXNlbGVjdCwgRXNjIGNhbmNlbHMgd2hlbiBhbGxvd2VkKS4KW25vdGlmaWNhdGlvbnNdCiMgTWFzdGVyIHN3aXRjaC4gSWYgZmFsc2UsIG5vdGlmaWNhdGlvbnMgYXJlIHNpbGVudGx5IGRyb3BwZWQgYXQKIyBhcnJpdmFsIOKAlCBhcHBzIHN0aWxsIHNlbmQgdGhlbSwgdGhlIG1vZGFsIG5ldmVyIGFwcGVhcnMsIGFuZAojIHRoZSBxdWV1ZSBzdGF5cyBlbXB0eS4KIyBlbmFibGVkID0gdHJ1ZQoKIyBGb2N1cyBtb2RlLiBXaGVuIHRydWUsIE5PIG5vdGlmaWNhdGlvbiBhdXRvLXN1cmZhY2VzIHJlZ2FyZGxlc3Mgb2YKIyBwcmlvcml0eS4gRXZlcnl0aGluZyBxdWV1ZXMgc2lsZW50bHk7IG9wZW4gQ21kK1NoaWZ0K0EgdG8gcmV2aWV3LgojIGZvY3VzX21vZGUgPSBmYWxzZQoKIyBNaW5pbXVtIHByaW9yaXR5IHRoYXQgbWF5IGF1dG8tb3BlbiB0aGUgbW9kYWwuIE5vdGlmaWNhdGlvbnMgYmVsb3cKIyB0aGlzIHZhbHVlIHF1ZXVlIHNpbGVudGx5IChiYWRnZSB0aWNrcyBvbiB0aGUgdG9vbGJhciwgQ21kK1NoaWZ0K0EKIyByZXZlYWxzIHRoZW0pLiBBdCBvciBhYm92ZSBpdCwgYXJyaXZhbCBhdXRvLW9wZW5zIHRoZSBtb2RhbC4KIwojIFRpZXJzIChmcm9tIHBsZXhpX3Nkayk6CiMgICAwICAgPSBQUklPUklUWV9MT1cgICAgICAgKGJhY2tncm91bmQgaW5mbykKIyAgIDUwICA9IFBSSU9SSVRZX05PUk1BTCAgICAoc3RhbmRhcmQgY29uZmlybWF0aW9ucyDigJQgIm5vdGUgc2F2ZWQiKQojICAgMTAwID0gUFJJT1JJVFlfSElHSCAgICAgIChuZWVkcyBhdHRlbnRpb24gc29vbikKIyAgIDIwMCA9IFBSSU9SSVRZX0NSSVRJQ0FMICAoaW50ZXJydXB0LWxldmVsKQojCiMgRGVmYXVsdCA9IDEwMDogTk9STUFMIGFuZCBMT1cgcXVldWUgc2lsZW50bHksIEhJR0ggYW5kIENSSVRJQ0FMCiMgaW50ZXJydXB0LiBTZXQgdG8gMCB0byBhdXRvLW9wZW4gZXZlcnl0aGluZy4gU2V0IHRvIDIwMSB0byBtYXRjaAojIGZvY3VzX21vZGUgPSB0cnVlIChub3RoaW5nIGF1dG8tb3BlbnMpLgojIGludGVycnVwdF90aHJlc2hvbGQgPSAxMDAKCiMgRXNjIHZzIEVudGVyIG9uIHRoZSBtb2RhbDoKIyAgIEVudGVyIChvciBvcHRpb24tc2VsZWN0IC8gaW5wdXQtc3VibWl0KSA9IGFja25vd2xlZGdlLiBOb3RpZmljYXRpb24KIyAgICAgaXMgcmVtb3ZlZCBmcm9tIHRoZSBxdWV1ZSBhbmQgdGhlIGFwcCByZWNlaXZlcyBOb3RpZnlBY3Rpb24uCiMgICBFc2MgPSBkZWZlci4gTW9kYWwgY2xvc2VzIGJ1dCB0aGUgbm90aWZpY2F0aW9uIHN0YXlzIGluIHRoZSBxdWV1ZSDigJQKIyAgICAgb3BlbiBDbWQrU2hpZnQrQSBsYXRlciB0byBjb21lIGJhY2sgdG8gaXQuIE5vIE5vdGlmeUFjdGlvbiBkaXNwYXRjaGVkLgojICAgUmVxdWlyZWQgbm90aWZpY2F0aW9ucyAocmVxdWlyZWQgPSB0cnVlKSBjYW5ub3QgYmUgRXNjJ2QuCgpbdGhlbWVdCiMgVW5jb21tZW50IGFueSBsaW5lIGJlbG93IHRvIG92ZXJyaWRlIHRoZSBhY3RpdmUgcHJlc2V0LgojIEZ1bGwgY29sb3IgbmFtZSByZWZlcmVuY2UgYW5kIGFsbCBwcmVzZXQgcGFsZXR0ZXMgKGNhdHBwdWNjaW4tbW9jaGEsCiMgZHJhY3VsYSwgdG9reW8tbmlnaHQsIGdydXZib3gtZGFyaywgbm9yZCwgc29sYXJpemVkLWRhcmspOgojIGh0dHBzOi8vcGxleGlhcHAuZGV2L2RvY3MvY29uZmlnCiMKIyBhY2NlbnQgICAgICAgPSAiIzg5YjRmYSIgICAjIGNhdHBwdWNjaW4tbW9jaGEgZGVmYXVsdHMgc2hvd24KIyBiZ19kYXJrZXN0ICAgPSAiIzExMTExYiIKIyB0ZXJtaW5hbF9iZyAgPSAiIzI5MmE0NCIKIyB0ZXh0X3ByaW1hcnkgPSAiI2NkZDZmNCIKIyBmb3JlZ3JvdW5kICAgPSAiI2U4ZTZlZCIKCiMg4pSA4pSAIFBsZXhpIEFJIOKAlCBjb21pbmcgc29vbiDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIAKIyBPbiB0aGUgcm9hZG1hcDogYXBwcyB3aWxsIGJlIGFibGUgdG8gbWFrZSB0aWVyLXJvdXRlZCBMTE0gY2FsbHMKIyB0aHJvdWdoIHRoZSBob3N0IGJyb2tlciB2aWEgdGhlIGBhaS5xdWVyeWAgY2FwYWJpbGl0eS4gV2hlbiB0aGF0CiMgc2hpcHMsIGNvbmZpZ3VyZSB5b3VyIGJhY2tlbmQgaGVyZS4KIwojIFthaV0KIyBiYWNrZW5kID0gIm9wZW5yb3V0ZXIiICAgIyAib3BlbnJvdXRlciIgKGNsb3VkKSBvciAib2xsYW1hIiAobG9jYWwpCiMKIyBbYWkub3BlbnJvdXRlcl0KIyBhcGlfa2V5X2VudiAgPSAiT1BFTlJPVVRFUl9BUElfS0VZIiAgICMgZXhwb3J0IGluIH4vLnpwcm9maWxlCiMgbW9kZWxfbG93ICAgID0gImdvb2dsZS9nZW1pbmktMi4wLWZsYXNoLTAwMSIKIyBtb2RlbF9tZWRpdW0gPSAiYW50aHJvcGljL2NsYXVkZS1zb25uZXQtNC02IgojIG1vZGVsX2hpZ2ggICA9ICJhbnRocm9waWMvY2xhdWRlLW9wdXMtNC03IgojCiMgW2FpLm9sbGFtYV0KIyBob3N0ICAgICAgICAgPSAiaHR0cDovL2xvY2FsaG9zdDoxMTQzNCIKIyBtb2RlbF9sb3cgICAgPSAibGxhbWEzLjI6M2IiCiMgbW9kZWxfbWVkaXVtID0gImxsYW1hMy4zOjcwYiIKIyBtb2RlbF9oaWdoICAgPSAicXdxOjMyYiIKCiMg4pSA4pSAIEV4cGVyaW1lbnRhbCBGZWF0dXJlcyDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIAKIyBGbGlwIGFueSBmbGFnIHRvIHRydWUgYW5kIHJlc3RhcnQgdG8gZW5hYmxlLgpbYmV0YV0KIyBjcnQgICA9IGZhbHNlICAgICMgUmV0cm8gQ1JUIHNjYW5saW5lcyArIGdyZWVuIHBob3NwaG9yIHRpbnQKIyBnaG9zdCA9IGZhbHNlICAgICMgVW5mb2N1c2VkIHBhbmVzIHJlbmRlciBhdCByZWR1Y2VkIG9wYWNpdHkKCiMg4pSA4pSAIExvZ2dpbmcg4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSACiMgW2xvZ10KIyBsZXZlbCA9ICJpbmZvIiAgICAgICAgICAjIGVycm9yIHwgd2FybiB8IGluZm8gfCBkZWJ1ZyAgKGRlZmF1bHQ6IGluZm8pCiMgcmV0ZW50aW9uX2RheXMgPSAzMCAgICAgIyBkYXlzIHRvIGtlZXAgZGF0ZWQgbG9nIGFyY2hpdmVzIChkZWZhdWx0OiAzMCkKCiMg4pSA4pSAIEtleWJpbmRpbmdzIOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgAojIE92ZXJyaWRlIGFueSBkZWZhdWx0IGtleWJpbmRpbmcuIEZvcm1hdDogIm1vZGlmaWVyK2tleSIgKGNhc2UtaW5zZW5zaXRpdmUpLgojIE1vZGlmaWVyczogY21kLCBzaGlmdCwgY3RybCwgYWx0IChhbGlhc2VzOiBjb21tYW5kLCBjb250cm9sLCBvcHQsIG9wdGlvbikuCiMgS2V5czogYS16LCAwLTksIGVudGVyLCBlc2NhcGUsIHRhYiwgc3BhY2UsIGJhY2tzcGFjZSwgdXAsIGRvd24sCiMgICBsZWZ0LCByaWdodCwgb3Blbl9icmFja2V0LCBjbG9zZV9icmFja2V0LCBiYWNrc2xhc2gsIHNsYXNoLCBjb21tYSwKIyAgIHBlcmlvZCwgZXF1YWxzLCBtaW51cy4KIyBVbmtub3duIGtleXMgb3IgY29uZmxpY3Rpbmcgb3ZlcnJpZGVzIGxvZyBhbiBlcnJvciBhdCBzdGFydHVwLgojIFtrZXliaW5kaW5nc10KIyBxdWl0ICAgICAgICAgICAgICAgICAgICA9ICJjbWQrcSIKIyBjbG9zZV9wYW5lICAgICAgICAgICAgICA9ICJjbWQrdyIKIyB0b2dnbGVfY29tbWFuZF9wYWxldHRlICA9ICJjbWQrcCIKIyBzcGxpdF9ob3Jpem9udGFsICAgICAgICA9ICJjbWQrZCIKIyBzcGxpdF92ZXJ0aWNhbCAgICAgICAgICA9ICJjbWQrc2hpZnQrZCIKIyBzcGxpdF9yaWdodCAgICAgICAgICAgICA9ICJjbWQrYmFja3NsYXNoIgojIHNwbGl0X2Rvd24gICAgICAgICAgICAgID0gImNtZCtzaGlmdCtiYWNrc2xhc2giCiMgbmF2aWdhdGVfbGVmdCAgICAgICAgICAgPSAiY21kK2giCiMgbmF2aWdhdGVfZG93biAgICAgICAgICAgPSAiY21kK2oiCiMgbmF2aWdhdGVfdXAgICAgICAgICAgICAgPSAiY21kK2siCiMgbmF2aWdhdGVfcmlnaHQgICAgICAgICAgPSAiY21kK2wiCiMgc3dhcF9wYW5lX2xlZnQgICAgICAgICAgPSAiY21kK2N0cmwraCIKIyBzd2FwX3BhbmVfZG93biAgICAgICAgICA9ICJjbWQrY3RybCtqIgojIHN3YXBfcGFuZV91cCAgICAgICAgICAgID0gImNtZCtjdHJsK2siCiMgc3dhcF9wYW5lX3JpZ2h0ICAgICAgICAgPSAiY21kK2N0cmwrbCIKIyBuZXdfdGFiICAgICAgICAgICAgICAgICA9ICJjbWQrdCIKIyBuZXh0X3RhYiAgICAgICAgICAgICAgICA9ICJjbWQrc2hpZnQrbCIKIyBwcmV2X3RhYiAgICAgICAgICAgICAgICA9ICJjbWQrc2hpZnQraCIKIyBmaXJzdF90YWIgICAgICAgICAgICAgICA9ICJjbWQrc2hpZnQrayIKIyBsYXN0X3RhYiAgICAgICAgICAgICAgICA9ICJjbWQrc2hpZnQraiIKIyBuYXZfYmFjayAgICAgICAgICAgICAgICA9ICJjbWQrb3Blbl9icmFja2V0IgojIHRvZ2dsZV9zaWRlYmFyICAgICAgICAgID0gImNtZCtiIgojIHRvZ2dsZV96b29tICAgICAgICAgICAgID0gImNtZCtlbnRlciIKIyB0b2dnbGVfc2hvcnRjdXRzICAgICAgICA9ICJjbWQrc2xhc2giCiMgcmVuYW1lX3BhbmUgICAgICAgICAgICAgPSAiY21kK3IiCiMgcmVuYW1lX2NvbnRleHQgICAgICAgICAgPSAiY21kK3NoaWZ0K3IiCiMgbmV3X3BhZ2VfcmlnaHQgICAgICAgICAgPSAiY21kK24iCiMgbmV3X2NvbnRleHQgICAgICAgICAgICAgPSAiY21kK3NoaWZ0K24iCiMgdG9nZ2xlX21pbmltYXAgICAgICAgICAgPSAiY21kK3NoaWZ0K20iCiMgc2Nyb2xsX3VwICAgICAgICAgICAgICAgPSAiY21kK3VwIgojIHNjcm9sbF9kb3duICAgICAgICAgICAgID0gImNtZCtkb3duIgojIGluY3JlYXNlX2ZvbnRfc2l6ZSAgICAgID0gImNtZCtlcXVhbHMiCiMgZGVjcmVhc2VfZm9udF9zaXplICAgICAgPSAiY21kK21pbnVzIgojIG9wZW5fZmlsZV9icm93c2VyICAgICAgID0gImNtZCtlIgojIG9wZW5fcXVpY2tfbm90ZSAgICAgICAgID0gImNtZCswIgojIG9wZW5fY29uZmlnICAgICAgICAgICAgID0gImNtZCtjb21tYSIKIyByZWxvYWRfY29uZmlnICAgICAgICAgICA9ICJjbWQrc2hpZnQrY29tbWEiCiMgb3Blbl9zZWNyZXRzX21hbmFnZXIgICAgPSAiY21kK3NoaWZ0K3MiCiMgZm9yY2VfcmVsb2FkX2FwcCAgICAgICAgPSAiY21kK2FsdCtyIgojIHRvZ2dsZV9ub3RpZmljYXRpb25fbW9kYWwgPSAiY21kK3NoaWZ0K2EiCgojIOKUgOKUgCBRdWljayBOb3RlIOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgAojIENtZCswIG9wZW5zIGEgY29tcG9zZSBtb2RhbC4gRW50ZXIgYWR2YW5jZXMgdG8gdGhlIGRlc3RpbmF0aW9uCiMgcGlja2VyLiBQcmVzcyBhIGRpZ2l0IGtleSB0byByb3V0ZSBpbnN0YW50bHkg4oCUIG5vIEVudGVyIG5lZWRlZC4KIyBTdWJtZW51IGVudHJpZXMgKG9wdGlvbnMgPSBbLi4uXSkgZXhwYW5kIG9uIHRoZSBwYXJlbnQga2V5cHJlc3MuCiMgRGVzdGluYXRpb24gMCAoZ2xvYmFsIGJhY2tsb2cg4oaSIH4vLnBsZXhpL2JhY2tsb2cpIGlzIGFsd2F5cwojIGF2YWlsYWJsZSByZWdhcmRsZXNzIG9mIGNvbmZpZy4KIwojIHtub3RlfSA9IHlvdXIgbm90ZSB0ZXh0IChzaGVsbC1lc2NhcGVkKSAgIHtjd2R9ID0gZm9jdXNlZCBwYW5lIGRpcgoKW1txdWlja19ub3RlLmRlc3RpbmF0aW9uc11dCmtleSAgID0gMQpsYWJlbCA9ICJCYWNrbG9nIgp0eXBlICA9ICJiYWNrbG9nIgpwYXRoICA9ICJ+Ly5wbGV4aS9iYWNrbG9nIgoKW1txdWlja19ub3RlLmRlc3RpbmF0aW9uc11dCmtleSAgICAgID0gMgpsYWJlbCAgICA9ICJBc2sgQ2xhdWRlIgp0eXBlICAgICA9ICJwYW5lIgpjb21tYW5kICA9ICJjbGF1ZGUgLXAge25vdGV9Igpwb3NpdGlvbiA9ICJjb250ZXh0LWVuZCIKCltbcXVpY2tfbm90ZS5kZXN0aW5hdGlvbnNdXQprZXkgICA9IDMKbGFiZWwgPSAiR2l0SHViIGlzc3VlIgojIHtub3RlfSBiZWNvbWVzIHRoZSBpc3N1ZSB0aXRsZSBhbmQgYm9keS4gS2VlcCBxdWljayBub3RlcyBjb25jaXNlCiMgZm9yIHJlYWRhYmxlIHRpdGxlcy4gTGFiZWxzIG11c3QgZXhpc3Qgb24gdGhlIHJlcG8g4oCUIGJ1ZyBhbmQKIyBlbmhhbmNlbWVudCBhcmUgR2l0SHViIGRlZmF1bHRzLiBDcmVhdGUgY3VzdG9tIGxhYmVscyB3aXRoOgojICAgZ2ggbGFiZWwgY3JlYXRlICJteS1sYWJlbCIgLS1jb2xvciAiQUFBQUFBIgpvcHRpb25zID0gWwogIHsga2V5ID0gMSwgbGFiZWwgPSAiQnVnIiwgICAgICAgICBjb21tYW5kID0gImNkIHtjd2R9ICYmIGdoIGlzc3VlIGNyZWF0ZSAtLWxhYmVsIGJ1ZyAtLXRpdGxlIHtub3RlfSAtLWJvZHkge25vdGV9IiB9LAogIHsga2V5ID0gMiwgbGFiZWwgPSAiRW5oYW5jZW1lbnQiLCBjb21tYW5kID0gImNkIHtjd2R9ICYmIGdoIGlzc3VlIGNyZWF0ZSAtLWxhYmVsIGVuaGFuY2VtZW50IC0tdGl0bGUge25vdGV9IC0tYm9keSB7bm90ZX0iIH0sCiAgeyBrZXkgPSAzLCBsYWJlbCA9ICJObyBsYWJlbCIsICAgIGNvbW1hbmQgPSAiY2Qge2N3ZH0gJiYgZ2ggaXNzdWUgY3JlYXRlIC0tdGl0bGUge25vdGV9IC0tYm9keSB7bm90ZX0iIH0sCl0K" | base64 --decode > "$CONFIG"
  echo "config: created default config at $CONFIG — set OPENROUTER_API_KEY in your shell profile"
fi

# Ensure required top-level config sections are present (additive-only migration).
# [ai] is intentionally omitted — it's commented out in the template (coming soon).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
"$SCRIPT_DIR/migrate-config.sh" "$CONFIG" "[notifications]" "[theme]" "[beta]"

echo "Installed $app_dest"
echo "CLI: $bin_dest"
echo "Config dir: $profile_dir/"
echo "Apps: $(ls "$profile_dir/apps" | wc -l | tr -d ' ') synced from examples/"
