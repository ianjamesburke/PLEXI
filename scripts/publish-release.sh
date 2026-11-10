#!/usr/bin/env bash
# Releases remain drafts until every required, tested asset is attached.
set -euo pipefail
tag="${1:?release tag required}"
dist="${2:?artifact directory required}"
assets=()
alpha_only=false
[[ "$tag" != *-alpha.* ]] || alpha_only=true
platforms=(macos-arm64 linux-x64)
if [[ "$alpha_only" == false ]]; then platforms+=(windows-x64 macos-x64); fi
for platform in "${platforms[@]}"; do
  ext=tar.gz; exe=''
  if [[ "$platform" == windows-* ]]; then ext=zip; exe=.exe; fi
  names=("plexi-$platform.$ext" "plexi-$platform-alpha.$ext" "plexi-$platform-beta.$ext" "plexi-installer-$platform$exe")
  if [[ "$alpha_only" == true ]]; then
    names=("plexi-$platform-alpha.$ext" "plexi-installer-$platform$exe")
  fi
  valid=true
  for name in "${names[@]}"; do
    if [[ ! -s "$dist/$name" || ! -s "$dist/$name.sha256" ]]; then
      echo "Missing artifact for $platform: $name or checksum" >&2
      valid=false
      break
    fi
    if command -v sha256sum >/dev/null 2>&1; then
      if ! (cd "$dist" && sha256sum --check "$name.sha256"); then valid=false; break; fi
    else
      if ! (cd "$dist" && shasum -a 256 --check "$name.sha256"); then valid=false; break; fi
    fi
  done
  if [[ "$valid" == false ]]; then
    if [[ "$platform" == macos-arm64 ]]; then
      echo "Required platform $platform failed artifact verification" >&2
      exit 1
    fi
    echo "Skipping optional platform $platform: incomplete or invalid artifacts" >&2
    continue
  fi
  for name in "${names[@]}"; do assets+=("$dist/$name" "$dist/$name.sha256"); done
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
