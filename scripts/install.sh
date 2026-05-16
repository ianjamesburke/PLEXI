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
app_src="target/release/bundle/osx/Plexi.app"
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
cp -R sdk/python/plexi_sdk "$profile_dir/sdk/plexi_sdk.tmp"
rm -rf "$profile_dir/sdk/plexi_sdk.py" "$profile_dir/sdk/plexi_sdk"
mv "$profile_dir/sdk/plexi_sdk.tmp" "$profile_dir/sdk/plexi_sdk"
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
  echo "IyDilZTilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZDilZcKIyDilZEgIFBsZXhpIENvbmZpZ3VyYXRpb24gICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAg4pWRCiMg4pWRICBDaGFuZ2VzIHRha2UgZWZmZWN0IG9uIG5leHQgbGF1bmNoLiAgICAgICAgICAgICAgICAgICAgICAgIOKVkQojIOKVmuKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVkOKVnQoKZm9udF9zaXplID0gMTQuMAoKIyDilIDilIAgVGhlbWUg4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSACiMgUGljayBhIHByZXNldCBPUiBjdXN0b21pemUgaW5kaXZpZHVhbCBjb2xvcnMgYmVsb3cuCiMgUHJlc2V0czogY2F0cHB1Y2Npbi1tb2NoYSwgZHJhY3VsYSwgdG9reW8tbmlnaHQsIGdydXZib3gtZGFyaywgbm9yZCwgc29sYXJpemVkLWRhcmsKdGhlbWVfcHJlc2V0ID0gImNhdHBwdWNjaW4tbW9jaGEiCgojIOKUgOKUgCBDb25maXJtYXRpb24gRGlhbG9ncyDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIAKIyBjb25maXJtX3F1aXQgcmVxdWlyZXMgdHJpcGxlIENtZCtRIHRvIGV4aXQgKHNhZmVyIHRoYW4gYSBzaW5nbGUgcHJlc3MpLgojIGNvbmZpcm1fY2xvc2Ugc2hvd3MgYSBkaWFsb2cgYmVmb3JlIENtZCtXIGNsb3NlcyBhIHBhbmUuCmNvbmZpcm1fcXVpdCAgPSB0cnVlCmNvbmZpcm1fY2xvc2UgPSBmYWxzZQoKIyDilIDilIAgTm90aWZpY2F0aW9ucyDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIAKIyBUaGUgd29yay1hcmVhIG1vZGFsIGlzIHRoZSBvbmUgYW5kIG9ubHkgbm90aWZpY2F0aW9uIHN1cmZhY2UuCiMgQXBwcyBlbWl0IGBjdHgubm90aWZ5KC4uLilgLCBgY3R4Lm5vdGlmeV9jaG9pY2UoLi4uKWAsIG9yCiMgYGN0eC5ub3RpZnlfaW5wdXQoLi4uKWAgYW5kIHRoZSBtb2RhbCByZW5kZXJzIGVhY2gga2luZCB3aXRoCiMga2V5Ym9hcmQtZmlyc3QgbmF2aWdhdGlvbiAoRW50ZXIgY29uZmlybXMsIGovayBvciDihpHihpMgY3ljbGUKIyBvcHRpb25zLCAxLTkgZGlyZWN0LXNlbGVjdCwgRXNjIGNhbmNlbHMgd2hlbiBhbGxvd2VkKS4KW25vdGlmaWNhdGlvbnNdCiMgTWFzdGVyIHN3aXRjaC4gSWYgZmFsc2UsIG5vdGlmaWNhdGlvbnMgYXJlIHNpbGVudGx5IGRyb3BwZWQgYXQKIyBhcnJpdmFsIOKAlCBhcHBzIHN0aWxsIHNlbmQgdGhlbSwgdGhlIG1vZGFsIG5ldmVyIGFwcGVhcnMsIGFuZAojIHRoZSBxdWV1ZSBzdGF5cyBlbXB0eS4KIyBlbmFibGVkID0gdHJ1ZQoKIyBGb2N1cyBtb2RlLiBXaGVuIHRydWUsIE5PIG5vdGlmaWNhdGlvbiBhdXRvLXN1cmZhY2VzIHJlZ2FyZGxlc3Mgb2YKIyBwcmlvcml0eS4gRXZlcnl0aGluZyBxdWV1ZXMgc2lsZW50bHk7IG9wZW4gQ21kK1NoaWZ0K0EgdG8gcmV2aWV3LgojIGZvY3VzX21vZGUgPSBmYWxzZQoKIyBNaW5pbXVtIHByaW9yaXR5IHRoYXQgbWF5IGF1dG8tb3BlbiB0aGUgbW9kYWwuIE5vdGlmaWNhdGlvbnMgYmVsb3cKIyB0aGlzIHZhbHVlIHF1ZXVlIHNpbGVudGx5IChiYWRnZSB0aWNrcyBvbiB0aGUgdG9vbGJhciwgQ21kK1NoaWZ0K0EKIyByZXZlYWxzIHRoZW0pLiBBdCBvciBhYm92ZSBpdCwgYXJyaXZhbCBhdXRvLW9wZW5zIHRoZSBtb2RhbC4KIwojIFRpZXJzIChmcm9tIHBsZXhpX3Nkayk6CiMgICAwICAgPSBQUklPUklUWV9MT1cgICAgICAgKGJhY2tncm91bmQgaW5mbykKIyAgIDUwICA9IFBSSU9SSVRZX05PUk1BTCAgICAoc3RhbmRhcmQgY29uZmlybWF0aW9ucyDigJQgIm5vdGUgc2F2ZWQiKQojICAgMTAwID0gUFJJT1JJVFlfSElHSCAgICAgIChuZWVkcyBhdHRlbnRpb24gc29vbikKIyAgIDIwMCA9IFBSSU9SSVRZX0NSSVRJQ0FMICAoaW50ZXJydXB0LWxldmVsKQojCiMgRGVmYXVsdCA9IDEwMDogTk9STUFMIGFuZCBMT1cgcXVldWUgc2lsZW50bHksIEhJR0ggYW5kIENSSVRJQ0FMCiMgaW50ZXJydXB0LiBTZXQgdG8gMCB0byBhdXRvLW9wZW4gZXZlcnl0aGluZy4gU2V0IHRvIDIwMSB0byBtYXRjaAojIGZvY3VzX21vZGUgPSB0cnVlIChub3RoaW5nIGF1dG8tb3BlbnMpLgojIGludGVycnVwdF90aHJlc2hvbGQgPSAxMDAKCiMgRXNjIHZzIEVudGVyIG9uIHRoZSBtb2RhbDoKIyAgIEVudGVyIChvciBvcHRpb24tc2VsZWN0IC8gaW5wdXQtc3VibWl0KSA9IGFja25vd2xlZGdlLiBOb3RpZmljYXRpb24KIyAgICAgaXMgcmVtb3ZlZCBmcm9tIHRoZSBxdWV1ZSBhbmQgdGhlIGFwcCByZWNlaXZlcyBOb3RpZnlBY3Rpb24uCiMgICBFc2MgPSBkZWZlci4gTW9kYWwgY2xvc2VzIGJ1dCB0aGUgbm90aWZpY2F0aW9uIHN0YXlzIGluIHRoZSBxdWV1ZSDigJQKIyAgICAgb3BlbiBDbWQrU2hpZnQrQSBsYXRlciB0byBjb21lIGJhY2sgdG8gaXQuIE5vIE5vdGlmeUFjdGlvbiBkaXNwYXRjaGVkLgojICAgUmVxdWlyZWQgbm90aWZpY2F0aW9ucyAocmVxdWlyZWQgPSB0cnVlKSBjYW5ub3QgYmUgRXNjJ2QuCgpbdGhlbWVdCiMgVW5jb21tZW50IGFueSBsaW5lIGJlbG93IHRvIG92ZXJyaWRlIHRoZSBhY3RpdmUgcHJlc2V0LgojIEZ1bGwgY29sb3IgbmFtZSByZWZlcmVuY2UgYW5kIGFsbCBwcmVzZXQgcGFsZXR0ZXMgKGNhdHBwdWNjaW4tbW9jaGEsCiMgZHJhY3VsYSwgdG9reW8tbmlnaHQsIGdydXZib3gtZGFyaywgbm9yZCwgc29sYXJpemVkLWRhcmspOgojIGh0dHBzOi8vcGxleGlhcHAuZGV2L2RvY3MvY29uZmlnCiMKIyBhY2NlbnQgICAgICAgPSAiIzg5YjRmYSIgICAjIGNhdHBwdWNjaW4tbW9jaGEgZGVmYXVsdHMgc2hvd24KIyBiZ19kYXJrZXN0ICAgPSAiIzExMTExYiIKIyB0ZXJtaW5hbF9iZyAgPSAiIzI5MmE0NCIKIyB0ZXh0X3ByaW1hcnkgPSAiI2NkZDZmNCIKIyBmb3JlZ3JvdW5kICAgPSAiI2U4ZTZlZCIKCiMg4pSA4pSAIFBsZXhpIEFJIOKAlCBjb21pbmcgc29vbiDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIAKIyBPbiB0aGUgcm9hZG1hcDogYXBwcyB3aWxsIGJlIGFibGUgdG8gbWFrZSB0aWVyLXJvdXRlZCBMTE0gY2FsbHMKIyB0aHJvdWdoIHRoZSBob3N0IGJyb2tlciB2aWEgdGhlIGBhaS5xdWVyeWAgY2FwYWJpbGl0eS4gV2hlbiB0aGF0CiMgc2hpcHMsIGNvbmZpZ3VyZSB5b3VyIGJhY2tlbmQgaGVyZS4KIwojIFthaV0KIyBiYWNrZW5kID0gIm9wZW5yb3V0ZXIiICAgIyAib3BlbnJvdXRlciIgKGNsb3VkKSBvciAib2xsYW1hIiAobG9jYWwpCiMKIyBbYWkub3BlbnJvdXRlcl0KIyBhcGlfa2V5X2VudiAgPSAiT1BFTlJPVVRFUl9BUElfS0VZIiAgICMgZXhwb3J0IGluIH4vLnpwcm9maWxlCiMgbW9kZWxfbG93ICAgID0gImdvb2dsZS9nZW1pbmktMi4wLWZsYXNoLTAwMSIKIyBtb2RlbF9tZWRpdW0gPSAiYW50aHJvcGljL2NsYXVkZS1zb25uZXQtNC02IgojIG1vZGVsX2hpZ2ggICA9ICJhbnRocm9waWMvY2xhdWRlLW9wdXMtNC03IgojCiMgW2FpLm9sbGFtYV0KIyBob3N0ICAgICAgICAgPSAiaHR0cDovL2xvY2FsaG9zdDoxMTQzNCIKIyBtb2RlbF9sb3cgICAgPSAibGxhbWEzLjI6M2IiCiMgbW9kZWxfbWVkaXVtID0gImxsYW1hMy4zOjcwYiIKIyBtb2RlbF9oaWdoICAgPSAicXdxOjMyYiIKCiMg4pSA4pSAIEV4cGVyaW1lbnRhbCBGZWF0dXJlcyDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIAKIyBGbGlwIGFueSBmbGFnIHRvIHRydWUgYW5kIHJlc3RhcnQgdG8gZW5hYmxlLgpbYmV0YV0KIyBjcnQgICA9IGZhbHNlICAgICMgUmV0cm8gQ1JUIHNjYW5saW5lcyArIGdyZWVuIHBob3NwaG9yIHRpbnQKIyBnaG9zdCAgICAgICAgID0gZmFsc2UgICAgIyBVbmZvY3VzZWQgcGFuZXMgcmVuZGVyIGF0IHJlZHVjZWQgb3BhY2l0eQojIG9zY19wYW5lX3RpdGxlID0gZmFsc2UgICAjIEFwcGx5IE9TQyAwLzEvMiB0aXRsZSBzZXF1ZW5jZXMgZnJvbSBydW5uaW5nIHByb2Nlc3NlcyBhcyBwYW5lIG5hbWVzCgojIOKUgOKUgCBMb2dnaW5nIOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgAojIFtsb2ddCiMgbGV2ZWwgPSAiaW5mbyIgICAgICAgICAgIyBlcnJvciB8IHdhcm4gfCBpbmZvIHwgZGVidWcgIChkZWZhdWx0OiBpbmZvKQojIHJldGVudGlvbl9kYXlzID0gMzAgICAgICMgZGF5cyB0byBrZWVwIGRhdGVkIGxvZyBhcmNoaXZlcyAoZGVmYXVsdDogMzApCgojIOKUgOKUgCBLZXliaW5kaW5ncyDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIAKIyBPdmVycmlkZSBhbnkgZGVmYXVsdCBrZXliaW5kaW5nLiBGb3JtYXQ6ICJtb2RpZmllcitrZXkiIChjYXNlLWluc2Vuc2l0aXZlKS4KIyBNb2RpZmllcnM6IGNtZCwgc2hpZnQsIGN0cmwsIGFsdCAoYWxpYXNlczogY29tbWFuZCwgY29udHJvbCwgb3B0LCBvcHRpb24pLgojIEtleXM6IGEteiwgMC05LCBlbnRlciwgZXNjYXBlLCB0YWIsIHNwYWNlLCBiYWNrc3BhY2UsIHVwLCBkb3duLAojICAgbGVmdCwgcmlnaHQsIG9wZW5fYnJhY2tldCwgY2xvc2VfYnJhY2tldCwgYmFja3NsYXNoLCBzbGFzaCwgY29tbWEsCiMgICBwZXJpb2QsIGVxdWFscywgbWludXMuCiMgVW5rbm93biBrZXlzIG9yIGNvbmZsaWN0aW5nIG92ZXJyaWRlcyBsb2cgYW4gZXJyb3IgYXQgc3RhcnR1cC4KIyBba2V5YmluZGluZ3NdCiMgcXVpdCAgICAgICAgICAgICAgICAgICAgPSAiY21kK3EiCiMgY2xvc2VfcGFuZSAgICAgICAgICAgICAgPSAiY21kK3ciCiMgdG9nZ2xlX2NvbW1hbmRfcGFsZXR0ZSAgPSAiY21kK3AiCiMgc3BsaXRfaG9yaXpvbnRhbCAgICAgICAgPSAiY21kK2QiCiMgc3BsaXRfdmVydGljYWwgICAgICAgICAgPSAiY21kK3NoaWZ0K2QiCiMgc3BsaXRfcmlnaHQgICAgICAgICAgICAgPSAiY21kK2JhY2tzbGFzaCIKIyBzcGxpdF9kb3duICAgICAgICAgICAgICA9ICJjbWQrc2hpZnQrYmFja3NsYXNoIgojIG5hdmlnYXRlX2xlZnQgICAgICAgICAgID0gImNtZCtoIgojIG5hdmlnYXRlX2Rvd24gICAgICAgICAgID0gImNtZCtqIgojIG5hdmlnYXRlX3VwICAgICAgICAgICAgID0gImNtZCtrIgojIG5hdmlnYXRlX3JpZ2h0ICAgICAgICAgID0gImNtZCtsIgojIHN3YXBfcGFuZV9sZWZ0ICAgICAgICAgID0gImNtZCtjdHJsK2giCiMgc3dhcF9wYW5lX2Rvd24gICAgICAgICAgPSAiY21kK2N0cmwraiIKIyBzd2FwX3BhbmVfdXAgICAgICAgICAgICA9ICJjbWQrY3RybCtrIgojIHN3YXBfcGFuZV9yaWdodCAgICAgICAgID0gImNtZCtjdHJsK2wiCiMgbmV3X3RhYiAgICAgICAgICAgICAgICAgPSAiY21kK3QiCiMgbmV4dF90YWIgICAgICAgICAgICAgICAgPSAiY21kK3NoaWZ0K2wiCiMgcHJldl90YWIgICAgICAgICAgICAgICAgPSAiY21kK3NoaWZ0K2giCiMgZmlyc3RfdGFiICAgICAgICAgICAgICAgPSAiY21kK3NoaWZ0K2siCiMgbGFzdF90YWIgICAgICAgICAgICAgICAgPSAiY21kK3NoaWZ0K2oiCiMgbmF2X2JhY2sgICAgICAgICAgICAgICAgPSAiY21kK29wZW5fYnJhY2tldCIKIyB0b2dnbGVfc2lkZWJhciAgICAgICAgICA9ICJjbWQrYiIKIyB0b2dnbGVfem9vbSAgICAgICAgICAgICA9ICJjbWQrZW50ZXIiCiMgdG9nZ2xlX3Nob3J0Y3V0cyAgICAgICAgPSAiY21kK3NsYXNoIgojIHJlbmFtZV9wYW5lICAgICAgICAgICAgID0gImNtZCtyIgojIHJlbmFtZV9jb250ZXh0ICAgICAgICAgID0gImNtZCtzaGlmdCtyIgojIG5ld19wYWdlX3JpZ2h0ICAgICAgICAgID0gImNtZCtuIgojIG5ld19jb250ZXh0ICAgICAgICAgICAgID0gImNtZCtzaGlmdCtuIgojIHRvZ2dsZV9taW5pbWFwICAgICAgICAgID0gImNtZCtzaGlmdCttIgojIHNjcm9sbF91cCAgICAgICAgICAgICAgID0gImNtZCt1cCIKIyBzY3JvbGxfZG93biAgICAgICAgICAgICA9ICJjbWQrZG93biIKIyBpbmNyZWFzZV9mb250X3NpemUgICAgICA9ICJjbWQrZXF1YWxzIgojIGRlY3JlYXNlX2ZvbnRfc2l6ZSAgICAgID0gImNtZCttaW51cyIKIyBvcGVuX2ZpbGVfYnJvd3NlciAgICAgICA9ICJjbWQrZSIKIyBvcGVuX3F1aWNrX25vdGUgICAgICAgICA9ICJjbWQrMCIKIyBvcGVuX2NvbmZpZyAgICAgICAgICAgICA9ICJjbWQrY29tbWEiCiMgcmVsb2FkX2NvbmZpZyAgICAgICAgICAgPSAiY21kK3NoaWZ0K2NvbW1hIgojIG9wZW5fc2VjcmV0c19tYW5hZ2VyICAgID0gImNtZCtzaGlmdCtzIgojIGZvcmNlX3JlbG9hZF9hcHAgICAgICAgID0gImNtZCthbHQrciIKIyB0b2dnbGVfbm90aWZpY2F0aW9uX21vZGFsID0gImNtZCtzaGlmdCthIgoKIyDilIDilIAgUXVpY2sgTm90ZSDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIAKIyBDbWQrMCBvcGVucyBhIGNvbXBvc2UgbW9kYWwuIEVudGVyIGFkdmFuY2VzIHRvIHRoZSBkZXN0aW5hdGlvbgojIHBpY2tlci4gUHJlc3MgYSBkaWdpdCBrZXkgdG8gcm91dGUgaW5zdGFudGx5IOKAlCBubyBFbnRlciBuZWVkZWQuCiMgU3VibWVudSBlbnRyaWVzIChvcHRpb25zID0gWy4uLl0pIGV4cGFuZCBvbiB0aGUgcGFyZW50IGtleXByZXNzLgojIERlc3RpbmF0aW9uIDAgKGdsb2JhbCBiYWNrbG9nIOKGkiB+Ly5wbGV4aS9iYWNrbG9nKSBpcyBhbHdheXMKIyBhdmFpbGFibGUgcmVnYXJkbGVzcyBvZiBjb25maWcuCiMKIyB7bm90ZX0gPSB5b3VyIG5vdGUgdGV4dCAoc2hlbGwtZXNjYXBlZCkgICB7Y3dkfSA9IGZvY3VzZWQgcGFuZSBkaXIKCltbcXVpY2tfbm90ZS5kZXN0aW5hdGlvbnNdXQprZXkgICA9IDEKbGFiZWwgPSAiQmFja2xvZyIKdHlwZSAgPSAiYmFja2xvZyIKcGF0aCAgPSAifi8ucGxleGkvYmFja2xvZyIKCltbcXVpY2tfbm90ZS5kZXN0aW5hdGlvbnNdXQprZXkgICAgICA9IDIKbGFiZWwgICAgPSAiQXNrIENsYXVkZSIKdHlwZSAgICAgPSAicGFuZSIKY29tbWFuZCAgPSAiY2xhdWRlIC1wIHtub3RlfSIKcG9zaXRpb24gPSAiY29udGV4dC1lbmQiCgpbW3F1aWNrX25vdGUuZGVzdGluYXRpb25zXV0Ka2V5ICAgPSAzCmxhYmVsID0gIkdpdEh1YiBpc3N1ZSIKIyB7bm90ZX0gYmVjb21lcyB0aGUgaXNzdWUgdGl0bGUgYW5kIGJvZHkuIEtlZXAgcXVpY2sgbm90ZXMgY29uY2lzZQojIGZvciByZWFkYWJsZSB0aXRsZXMuIExhYmVscyBtdXN0IGV4aXN0IG9uIHRoZSByZXBvIOKAlCBidWcgYW5kCiMgZW5oYW5jZW1lbnQgYXJlIEdpdEh1YiBkZWZhdWx0cy4gQ3JlYXRlIGN1c3RvbSBsYWJlbHMgd2l0aDoKIyAgIGdoIGxhYmVsIGNyZWF0ZSAibXktbGFiZWwiIC0tY29sb3IgIkFBQUFBQSIKb3B0aW9ucyA9IFsKICB7IGtleSA9IDEsIGxhYmVsID0gIkJ1ZyIsICAgICAgICAgY29tbWFuZCA9ICJjZCB7Y3dkfSAmJiBnaCBpc3N1ZSBjcmVhdGUgLS1sYWJlbCBidWcgLS10aXRsZSB7bm90ZX0gLS1ib2R5IHtub3RlfSIgfSwKICB7IGtleSA9IDIsIGxhYmVsID0gIkVuaGFuY2VtZW50IiwgY29tbWFuZCA9ICJjZCB7Y3dkfSAmJiBnaCBpc3N1ZSBjcmVhdGUgLS1sYWJlbCBlbmhhbmNlbWVudCAtLXRpdGxlIHtub3RlfSAtLWJvZHkge25vdGV9IiB9LAogIHsga2V5ID0gMywgbGFiZWwgPSAiTm8gbGFiZWwiLCAgICBjb21tYW5kID0gImNkIHtjd2R9ICYmIGdoIGlzc3VlIGNyZWF0ZSAtLXRpdGxlIHtub3RlfSAtLWJvZHkge25vdGV9IiB9LApdCg==" | base64 --decode > "$CONFIG"
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
