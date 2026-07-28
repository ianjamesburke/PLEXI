#!/usr/bin/env bash
# Version-lockstep gate for the published agent skill (stint 0570).
# The plexi-cli SKILL.md documents a specific CLI surface; shipping a stable
# release whose binary version differs from the skill's declared plexi_version
# means the published skill is stale (or ahead). This check FAILS the release.
#
# Usage: check-skill-version.sh <tree>
#   <tree> — repo checkout to validate (promote.sh passes the beta worktree).
set -euo pipefail

tree="${1:?usage: check-skill-version.sh <tree>}"
skill_md="$tree/skills/plexi-cli/SKILL.md"
cargo_toml="$tree/Cargo.toml"

[[ -f "$skill_md" ]] || { echo "error: $skill_md not found" >&2; exit 1; }
[[ -f "$cargo_toml" ]] || { echo "error: $cargo_toml not found" >&2; exit 1; }

binary_version=$(grep '^version' "$cargo_toml" | head -1 | sed 's/version = "\(.*\)"/\1/')
skill_version=$(awk -F'"' '/^plexi_version:/ { print $2; exit }' "$skill_md")

[[ -n "$binary_version" ]] || { echo "error: no version in $cargo_toml" >&2; exit 1; }
[[ -n "$skill_version" ]] || { echo "error: no plexi_version frontmatter in $skill_md" >&2; exit 1; }

if [[ "$skill_version" != "$binary_version" ]]; then
    echo "error: skill/binary version mismatch — release blocked" >&2
    echo "  binary (Cargo.toml):            $binary_version" >&2
    echo "  skill (SKILL.md plexi_version): $skill_version" >&2
    echo "  Re-verify skills/plexi-cli/SKILL.md against the current CLI, set" >&2
    echo "  plexi_version to $binary_version, and republish the mirror" >&2
    echo "  (see skills/AGENTS.md for the publish flow)." >&2
    exit 1
fi

echo "skill version lockstep OK: $skill_version"
