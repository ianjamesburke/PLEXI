#!/usr/bin/env bash
# Usage: scripts/install.sh [channel]
# Reads .channel from CWD if no argument given. Must be run from the repo root.
set -euo pipefail

if [[ "$(uname)" != "Darwin" ]]; then
  echo "install is macOS-only."
  exit 1
fi

channel="${1:-$(cat .channel 2>/dev/null || echo "stable")}"

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
  echo "W2FpXQpiYWNrZW5kID0gIm9wZW5yb3V0ZXIiCgpbYWkub3BlbnJvdXRlcl0KYXBpX2tleV9lbnYgPSAiT1BFTlJPVVRFUl9BUElfS0VZIgptb2RlbF9sb3cgICAgPSAiZ29vZ2xlL2dlbWluaS0yLjAtZmxhc2gtMDAxIgptb2RlbF9tZWRpdW0gPSAiYW50aHJvcGljL2NsYXVkZS1zb25uZXQtNC02Igptb2RlbF9oaWdoICAgPSAiYW50aHJvcGljL2NsYXVkZS1vcHVzLTQtNyIKClthaS5vbGxhbWFdCmhvc3QgICAgICAgICA9ICJodHRwOi8vbG9jYWxob3N0OjExNDM0Igptb2RlbF9sb3cgICAgPSAibGxhbWEzLjI6M2IiCm1vZGVsX21lZGl1bSA9ICJsbGFtYTMuMzo3MGIiCm1vZGVsX2hpZ2ggICA9ICJxd3E6MzJiIgo=" | base64 --decode > "$CONFIG"
  echo "config: created default config at $CONFIG — set OPENROUTER_API_KEY in your shell profile"
fi

# Ensure all required top-level config sections are present (additive-only migration).
# Sections: [notifications] [theme] [ai] [beta]
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
"$SCRIPT_DIR/migrate-config.sh" "$CONFIG" "[notifications]" "[theme]" "[ai]" "[beta]"

echo "Installed $app_dest"
echo "CLI: $bin_dest"
echo "Config dir: $profile_dir/"
echo "Apps: $(ls "$profile_dir/apps" | wc -l | tr -d ' ') synced from examples/"
