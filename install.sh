#!/usr/bin/env bash
# Usage:
#   curl -fsSL https://plexiapp.com/install | bash
#   curl -fsSL https://plexiapp.com/install | bash -s -- --channel alpha
#   curl -fsSL https://plexiapp.com/install | bash -s -- --channel beta
set -euo pipefail

# Keep the served file small and delegate to the versioned installer on the
# selected release branch. The delegated script is standalone and downloads a
# release asset; it never clones or builds the Plexi source tree by default.
exec bash -c 'curl -fsSL https://raw.githubusercontent.com/ianjamesburke/PLEXI/main/scripts/install.sh | bash -s -- "$@"' -- "$@"
