#!/usr/bin/env bash
# Versioned binary bootstrap. The release installer owns selection and activation.
set -euo pipefail
case "$(uname -s)" in
  Darwin) os=macos ;;
  Linux) os=linux ;;
  *) echo 'Use https://plexiapp.com/install.ps1 from PowerShell on Windows.' >&2; exit 1 ;;
esac
case "$(uname -m)" in
  arm64|aarch64) arch=arm64 ;;
  x86_64|amd64) arch=x64 ;;
  *) echo 'Unsupported processor architecture.' >&2; exit 1 ;;
esac
base="${PLEXI_RELEASE_BASE_URL:-https://github.com/ianjamesburke/PLEXI/releases/download}"
releases_url="${PLEXI_RELEASES_URL:-https://api.github.com/repos/ianjamesburke/PLEXI/releases}"

# Last --channel wins. The original argv is forwarded unchanged.
channel=stable
previous=""
for arg in "$@"; do
  if [[ "$previous" == "--channel" ]]; then
    channel="$arg"
  elif [[ "$arg" == --channel=* ]]; then
    channel="${arg#--channel=}"
  fi
  previous="$arg"
done
if [[ "$previous" == "--channel" || -z "$channel" ]]; then
  echo 'Missing value for --channel.' >&2
  exit 1
fi
case "$channel" in
  stable|main|alpha|beta|pr-*) ;;
  *) echo "Unsupported install channel: $channel" >&2; exit 1 ;;
esac

# SemVer, with a final release ordered above its prerelease. Bash 3.2 safe.
version_gt() {
  local a_raw="${1#v}" b_raw="${2#v}"
  local a_core="${a_raw%%-*}" b_core="${b_raw%%-*}"
  local a_pre="" b_pre=""
  [[ "$a_raw" == *-* ]] && a_pre="${a_raw#*-}"
  [[ "$b_raw" == *-* ]] && b_pre="${b_raw#*-}"
  local a_maj a_min a_pat b_maj b_min b_pat
  IFS=. read -r a_maj a_min a_pat <<<"$a_core"
  IFS=. read -r b_maj b_min b_pat <<<"$b_core"
  if (( a_maj > b_maj )); then return 0; fi
  if (( a_maj < b_maj )); then return 1; fi
  if (( a_min > b_min )); then return 0; fi
  if (( a_min < b_min )); then return 1; fi
  if (( a_pat > b_pat )); then return 0; fi
  if (( a_pat < b_pat )); then return 1; fi
  if [[ -z "$a_pre" && -z "$b_pre" ]]; then return 1; fi
  if [[ -z "$a_pre" ]]; then return 0; fi
  if [[ -z "$b_pre" ]]; then return 1; fi
  local a_lane="${a_pre%%.*}" b_lane="${b_pre%%.*}"
  local a_n="${a_pre#*.}" b_n="${b_pre#*.}"
  if [[ "$a_lane" != "$b_lane" ]]; then
    if [[ "$a_lane" == beta ]]; then return 0; fi
    return 1
  fi
  if (( a_n > b_n )); then return 0; fi
  return 1
}

max_semver() {
  local best="" tag
  while IFS= read -r tag; do
    [[ -z "$tag" ]] && continue
    if [[ -z "$best" ]] || version_gt "$tag" "$best"; then
      best="$tag"
    fi
  done
  printf '%s' "$best"
}

# GitHub emits compact JSON with tag_name before draft. Drafts and non-channel tags are dropped.
extract_tags() {
  grep -oE '"tag_name"[[:space:]]*:[[:space:]]*"[^"]+"|"draft"[[:space:]]*:[[:space:]]*(true|false)|"name"[[:space:]]*:[[:space:]]*"[^"]+"' | awk -v asset="$asset" '
    function flush() {
      if (have && !draft && has_asset && tag ~ /^v[0-9]+\.[0-9]+\.[0-9]+(-alpha\.[0-9]+|-beta\.[0-9]+)?$/) print tag
    }
    /"tag_name"/ {
      flush()
      tag = $0
      sub(/.*"tag_name"[[:space:]]*:[[:space:]]*"/, "", tag)
      sub(/".*/, "", tag)
      draft = 0
      has_asset = 0
      have = 1
      next
    }
    /"draft"/ { draft = ($0 ~ /true/) }
    /"name"/ {
      name = $0
      sub(/.*"name"[[:space:]]*:[[:space:]]*"/, "", name)
      sub(/".*/, "", name)
      if (name == asset) has_asset = 1
    }
    END { flush() }
  ' || true
}

fetch_tags() {
  local page=1 batch extracted count tags=""
  while (( page <= 1000 )); do
    batch="$(curl -q -fsSL --retry 3 --connect-timeout 15 "${releases_url}?per_page=100&page=${page}")"
    extracted="$(printf '%s' "$batch" | extract_tags)"
    if [[ -n "$extracted" ]]; then
      tags="${tags}${extracted}"$'\n'
    fi
    count="$(printf '%s' "$batch" | grep -o '"tag_name"' | wc -l | tr -d ' ')" || count=0
    [[ -n "$count" ]] || count=0
    if (( count < 100 )); then
      break
    fi
    if (( page == 1000 )); then
      echo 'release pagination exceeded its bound' >&2
      exit 1
    fi
    page=$((page + 1))
  done
  printf '%s' "$tags"
}

lane_tags() {
  local lane="$1" tag
  while IFS= read -r tag; do
    case "$lane" in
      alpha) [[ "$tag" == *-alpha.* ]] && printf '%s\n' "$tag" ;;
      beta) [[ "$tag" == *-beta.* ]] && printf '%s\n' "$tag" ;;
      stable) [[ "$tag" != *-* ]] && printf '%s\n' "$tag" ;;
    esac
  done
  return 0
}

# Mirrors release::select for the installer binary's tag. Alpha uses
# alpha_preference (alpha, then beta, then stable). Beta is plain SemVer
# across beta and stable. See crates/distribution/src/release.rs.
select_channel_tag() {
  local want="$1" tags best
  tags="$(fetch_tags)"
  if [[ "$want" == alpha ]]; then
    best="$(printf '%s\n' "$tags" | lane_tags alpha | max_semver)"
    [[ -n "$best" ]] && { printf '%s' "$best"; return; }
    best="$(printf '%s\n' "$tags" | lane_tags beta | max_semver)"
    [[ -n "$best" ]] && { printf '%s' "$best"; return; }
    printf '%s' "$(printf '%s\n' "$tags" | lane_tags stable | max_semver)"
    return
  fi
  if [[ "$want" == stable ]]; then
    printf '%s' "$(printf '%s\n' "$tags" | lane_tags stable | max_semver)"
    return
  fi
  best="$(printf '%s\n' "$tags" | lane_tags beta; printf '%s\n' "$tags" | lane_tags stable)"
  printf '%s\n' "$best" | max_semver
}

# Resolve once, then pin both executable and checksum to the same released tag.
# PLEXI_BOOTSTRAP_TAG overrides channel selection.
asset="plexi-installer-${os}-${arch}"
tag="${PLEXI_BOOTSTRAP_TAG:-}"
if [[ -z "$tag" ]]; then
  case "$channel" in
    alpha|beta) tag="$(select_channel_tag "$channel")" ;;
    *) tag="$(select_channel_tag stable)" ;;
  esac
fi
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-alpha\.[0-9]+|-beta\.[0-9]+)?$ ]] || { echo "No ${channel} release of Plexi is published for ${os}-${arch} yet." >&2; exit 1; }
echo "Resolved Plexi installer ${tag} (${channel})." >&2
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
if ! curl -q -fL --retry 3 --connect-timeout 15 "$base/$tag/$asset" -o "$work/$asset"; then
  echo "Installer asset ${asset} is not published for ${tag} (${os}-${arch})." >&2
  exit 1
fi
curl -q -fL --retry 3 --connect-timeout 15 "$base/$tag/$asset.sha256" -o "$work/checksum"
expected="$(awk -v name="$asset" '$2 == name || $2 == "*" name {print $1}' "$work/checksum")"
[[ "$expected" =~ ^[[:xdigit:]]{64}$ ]] || { echo 'Malformed installer checksum.' >&2; exit 1; }
if command -v sha256sum >/dev/null 2>&1; then actual="$(sha256sum "$work/$asset" | awk '{print $1}')"
else actual="$(shasum -a 256 "$work/$asset" | awk '{print $1}')"; fi
[[ "$actual" == "$expected" ]] || { echo 'Installer checksum mismatch.' >&2; exit 1; }
chmod 755 "$work/$asset"
"$work/$asset" "$@"
