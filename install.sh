#!/usr/bin/env bash
# Usage:
#   curl -fsSL https://plexiapp.com/install | bash
#   curl -fsSL https://plexiapp.com/install | bash -s -- --channel alpha
#   curl -fsSL https://plexiapp.com/install | bash -s -- --channel beta
set -euo pipefail

# Keep the served file small and delegate to the alpha binary installer. Until
# main publishes binary assets, the public command honestly defaults to alpha.
# The delegated script downloads a release asset; this wrapper never clones or
# builds the Plexi source tree.
has_channel=0
from_source=0
for arg in "$@"; do
  case "$arg" in
    --channel|-c|--channel=*|-c=*) has_channel=1 ;;
    --from-source) from_source=1 ;;
  esac
done

args=("$@")
if [[ "$from_source" == 1 ]]; then
  # The source-install compatibility path in scripts/install.sh keys off $1.
  args=(--from-source)
  for arg in "$@"; do
    [[ "$arg" == --from-source ]] || args+=("$arg")
  done
elif [[ "$has_channel" == 0 ]]; then
  args+=(--channel alpha)
fi

installer="$(mktemp)"
trap 'rm -f "$installer"' EXIT
curl -fsSL https://raw.githubusercontent.com/ianjamesburke/PLEXI/alpha/scripts/install.sh -o "$installer"
bash "$installer" "${args[@]}"
