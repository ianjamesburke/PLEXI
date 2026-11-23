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
# Resolution reads releases/latest, git/matching-refs/tags/v and releases/tags/<tag>,
# never the paginated release list (it returned 504 for hours on 2026-11-23).
repo_api="${PLEXI_GITHUB_REPO_API:-https://api.github.com/repos/ianjamesburke/PLEXI}"
max_tag_lookups=5
github_token="${GITHUB_TOKEN:-${GH_TOKEN:-}}"

# Last --channel and --tag win. The original argv is forwarded unchanged.
channel=stable
user_tag=""
previous=""
for arg in "$@"; do
  if [[ "$previous" == "--channel" ]]; then
    channel="$arg"
  elif [[ "$arg" == --channel=* ]]; then
    channel="${arg#--channel=}"
  elif [[ "$previous" == "--tag" ]]; then
    user_tag="$arg"
  elif [[ "$arg" == --tag=* ]]; then
    user_tag="${arg#--tag=}"
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

# Prints the tag of one release JSON object when it is not a draft and carries $asset.
# GitHub emits tag_name before draft and assets.
release_tag_with_asset() {
  grep -oE '"tag_name"[[:space:]]*:[[:space:]]*"[^"]+"|"draft"[[:space:]]*:[[:space:]]*(true|false)|"name"[[:space:]]*:[[:space:]]*"[^"]+"' | awk -v asset="$asset" '
    /"tag_name"/ {
      tag = $0
      sub(/.*"tag_name"[[:space:]]*:[[:space:]]*"/, "", tag)
      sub(/".*/, "", tag)
    }
    /"draft"/ { draft = ($0 ~ /true/) }
    /"name"/ {
      name = $0
      sub(/.*"name"[[:space:]]*:[[:space:]]*"/, "", name)
      sub(/".*/, "", name)
      if (name == asset) has_asset = 1
    }
    END { if (!draft && has_asset && tag ~ /^v[0-9]+\.[0-9]+\.[0-9]+(-alpha\.[0-9]+|-beta\.[0-9]+)?$/) print tag }
  ' || true
}

# GET <path> under the repository API into <file>; prints the HTTP status.
# Server errors, timeouts and resets are retried with doubling delays; a rate
# limit or other 4xx answers at once. Exhausted retries say GitHub is slow.
github_get() {
  local url="$repo_api/$1" out="$2" attempt=1 attempts=3 delay=1
  local headers status rc detail reset retry_after remaining
  while :; do
    headers="$(mktemp)"
    local curl_args=(-q -sS -L --connect-timeout 10 --max-time 20 -H 'Accept: application/vnd.github+json' -D "$headers" -o "$out" -w '%{http_code}')
    [[ -n "$github_token" ]] && curl_args+=(-H "Authorization: Bearer $github_token")
    rc=0
    status="$(curl "${curl_args[@]}" "$url" 2>/dev/null)" || rc=$?
    remaining="$(awk 'tolower($1) == "x-ratelimit-remaining:" {sub(/\r$/, "", $2); print $2}' "$headers" | tail -n 1)"
    reset="$(awk 'tolower($1) == "x-ratelimit-reset:" {sub(/\r$/, "", $2); print $2}' "$headers" | tail -n 1)"
    retry_after="$(awk 'tolower($1) == "retry-after:" {sub(/\r$/, "", $2); print $2}' "$headers" | tail -n 1)"
    rm -f "$headers"
    # A received status decides first: a 4xx or rate limit stands even when the
    # body then resets or times out (curl keeps the status and exits nonzero).
    [[ "$status" =~ ^[1-9][0-9][0-9]$ ]] || status=""
    if [[ "$status" == 429 || ( "$status" == 403 && ( "$remaining" == 0 || -n "$retry_after" ) ) ]]; then
      local reset_at=""
      if [[ "$reset" =~ ^[0-9]+$ ]]; then
        reset_at="$(date -d "@$reset" '+%Y-%m-%d %H:%M %Z' 2>/dev/null || date -r "$reset" '+%Y-%m-%d %H:%M %Z' 2>/dev/null || true)"
      elif [[ "$retry_after" =~ ^[0-9]+$ ]]; then
        local until=$(( $(date +%s) + retry_after ))
        reset_at="$(date -d "@$until" '+%Y-%m-%d %H:%M %Z' 2>/dev/null || date -r "$until" '+%Y-%m-%d %H:%M %Z' 2>/dev/null || true)"
      fi
      [[ -n "$reset_at" ]] || reset_at="the time shown by GitHub"
      echo "GitHub API rate limit reached (unauthenticated limit is 60 requests/hour per IP). Resets at ${reset_at}. Set GITHUB_TOKEN (or GH_TOKEN) and retry, or retry after the reset." >&2
      return 1
    fi
    if [[ -n "$status" ]] && (( status >= 300 && status < 500 )); then
      printf '%s' "$status"
      return 0
    fi
    if [[ -n "$status" ]] && (( status < 300 )) && (( rc == 0 )); then
      printf '%s' "$status"
      return 0
    fi
    # Left: a 5xx, or a transport failure before or during the body.
    case "$rc" in
      0) detail="HTTP $status" ;;
      7|28|35|52|55|56) detail="curl exit $rc${status:+ after HTTP $status}" ;;
      *) echo "Unable to reach GitHub at $url (curl exit $rc)." >&2; return 1 ;;
    esac
    if (( attempt >= attempts )); then
      echo "GitHub is slow, try again. ($detail from $url after $attempts attempts)" >&2
      return 1
    fi
    echo "GitHub request failed ($detail); retrying in ${delay}s (attempt $attempt/$attempts)." >&2
    sleep "$delay"
    attempt=$((attempt + 1))
    delay=$((delay * 2))
  done
}

# Prints the release's tag when <path> names a published release carrying $asset.
release_at() {
  local body status
  body="$(mktemp)"
  status="$(github_get "$1" "$body")" || { rm -f "$body"; return 1; }
  case "$status" in
    200) release_tag_with_asset <"$body" ;;
    404) ;;
    *) rm -f "$body"; echo "Unable to read Plexi release $1: HTTP ${status}." >&2; return 1 ;;
  esac
  rm -f "$body"
}

fetch_tags() {
  local body status
  body="$(mktemp)"
  status="$(github_get "git/matching-refs/tags/v" "$body")" || { rm -f "$body"; return 1; }
  if [[ "$status" != 200 ]]; then
    rm -f "$body"
    echo "Unable to list Plexi release tags: HTTP ${status}." >&2
    return 1
  fi
  grep -oE '"ref"[[:space:]]*:[[:space:]]*"refs/tags/[^"]+"' "$body" | sed -E 's#.*refs/tags/##; s#"$##' |
    grep -E '^v[0-9]+\.[0-9]+\.[0-9]+(-alpha\.[0-9]+|-beta\.[0-9]+)?$' || true
  rm -f "$body"
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

# The best <count> tags on stdin, best first.
top_tags() {
  local count="$1" tags best
  tags="$(cat)"
  while (( count > 0 )); do
    best="$(printf '%s\n' "$tags" | max_semver)"
    [[ -z "$best" ]] && break
    printf '%s\n' "$best"
    tags="$(printf '%s\n' "$tags" | grep -vxF "$best" || true)"
    count=$((count - 1))
  done
}

# Mirrors release::resolve (crates/distribution/src/release.rs). Stable is
# GitHub's latest release. Alpha ranks alpha, then beta, then stable tags; beta
# ranks beta and stable tags by SemVer. The best ranked tags are looked up one
# at a time and the first published release with the installer wins.
select_channel_tag() {
  local want="$1" tags ranked tag found count=0
  if [[ "$want" == stable ]]; then
    release_at "releases/latest"
    return
  fi
  tags="$(fetch_tags)" || return 1
  if [[ "$want" == alpha ]]; then
    ranked="$(
      printf '%s\n' "$tags" | lane_tags alpha | top_tags $((max_tag_lookups + 1))
      printf '%s\n' "$tags" | lane_tags beta | top_tags $((max_tag_lookups + 1))
      printf '%s\n' "$tags" | lane_tags stable | top_tags $((max_tag_lookups + 1))
    )"
  else
    ranked="$({ printf '%s\n' "$tags" | lane_tags beta; printf '%s\n' "$tags" | lane_tags stable; } | top_tags $((max_tag_lookups + 1)))"
  fi
  while IFS= read -r tag; do
    [[ -z "$tag" ]] && continue
    if (( count == max_tag_lookups )); then
      echo "None of the newest ${max_tag_lookups} ${want} releases has ${asset}." >&2
      return 1
    fi
    found="$(release_at "releases/tags/$tag")" || return 1
    if [[ -n "$found" ]]; then
      printf '%s' "$found"
      return
    fi
    count=$((count + 1))
  done <<<"$ranked"
}

# Resolve once, then pin both executable and checksum to the same released tag.
# PLEXI_BOOTSTRAP_TAG overrides channel selection.
# A user --tag pins the installer to that release and needs no lookup.
asset="plexi-installer-${os}-${arch}"
tag="${PLEXI_BOOTSTRAP_TAG:-$user_tag}"
if [[ -z "$tag" ]]; then
  case "$channel" in
    alpha|beta) tag="$(select_channel_tag "$channel")" || exit 1 ;;
    *) tag="$(select_channel_tag stable)" || exit 1 ;;
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
has_tag=0
previous=""
for arg in "$@"; do
  [[ "$previous" == "--tag" || "$arg" == --tag=* ]] && has_tag=1
  previous="$arg"
done
case "$channel" in
  stable|main|alpha|beta) forward_resolved_tag=1 ;;
  pr-*) forward_resolved_tag=0 ;;
esac
if (( has_tag || !forward_resolved_tag )); then
  "$work/$asset" "$@"
else
  "$work/$asset" "$@" --tag "$tag"
fi
