#!/usr/bin/env bash
# Releases remain drafts until every required, tested asset is attached.
set -euo pipefail
tag="${1:?release tag required}"
dist="${2:?artifact directory required}"
assets=()
platforms=(macos-arm64 linux-x64 windows-x64)
[[ ! -f "$dist/plexi-macos-x64.tar.gz" ]] || platforms+=(macos-x64)
for platform in "${platforms[@]}"; do
  ext=tar.gz; exe=''
  if [[ "$platform" == windows-* ]]; then ext=zip; exe=.exe; fi
  names=("plexi-$platform.$ext" "plexi-$platform-alpha.$ext" "plexi-$platform-beta.$ext" "plexi-installer-$platform$exe")
  for name in "${names[@]}"; do
    [[ -s "$dist/$name" && -s "$dist/$name.sha256" ]] || { echo "Required artifact missing: $name or checksum" >&2; exit 1; }
    (cd "$dist" && sha256sum --check "$name.sha256")
    assets+=("$dist/$name" "$dist/$name.sha256")
  done
done
existing="$(gh release view "$tag" --json isDraft --jq '.isDraft' 2>/dev/null || true)"
if [[ "$existing" == false ]]; then echo "Published release $tag is immutable. Cut a new version." >&2; exit 1; fi
prerelease=false; latest=true
if [[ "$tag" == *-* ]]; then prerelease=true; latest=false; fi
if [[ -z "$existing" ]]; then
  gh release create "$tag" --verify-tag --draft --title "$tag" --generate-notes --prerelease="$prerelease"
fi
# Never clobber existing assets, including on a retry. A failed draft can be
# inspected and explicitly cleaned before retrying; consumers never see it.
gh release upload "$tag" "${assets[@]}"
gh release edit "$tag" --draft=false --latest="$latest"
echo "Published $tag after package installation gates and asset verification."
