#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out_dir="${1:-/tmp/plexi-app-contact-sheet}"
size="${PLEXI_RENDER_SIZE:-1000x720}"
plexi_bin="${PLEXI_BIN:-$repo_root/target/debug/plexi}"

mkdir -p "$out_dir"

if [[ ! -x "$plexi_bin" ]]; then
  echo "error: plexi binary not found or not executable: $plexi_bin" >&2
  echo "build it first with: cargo build" >&2
  exit 1
fi

render_app() {
  local label="$1"
  local app_path="$2"
  local state_path="${3:-}"
  local png="$out_dir/${label}.png"
  local json="$out_dir/${label}.json"

  if [[ -n "$state_path" ]]; then
    PLEXI_SDK_PATH="$repo_root/sdk/python" "$plexi_bin" app render "$app_path" \
      --size "$size" --state "$state_path" --png --output "$png"
    PLEXI_SDK_PATH="$repo_root/sdk/python" "$plexi_bin" app render "$app_path" \
      --size "$size" --state "$state_path" --output "$json"
  else
    PLEXI_SDK_PATH="$repo_root/sdk/python" "$plexi_bin" app render "$app_path" \
      --size "$size" --png --output "$png"
    PLEXI_SDK_PATH="$repo_root/sdk/python" "$plexi_bin" app render "$app_path" \
      --size "$size" --output "$json"
  fi
}

write_github_state_fixtures() {
  cat > "$out_dir/github-list-state.json" <<'JSON'
{
  "repo": "plexi/app",
  "issues": [
    {
      "number": 285,
      "title": "SDK v3 native apps visual audit",
      "state": "open",
      "body": "Audit every core app render before merge.",
      "createdAt": "2026-06-23T15:00:00Z",
      "labels": [{"name": "p1"}, {"name": "sdk-v3"}, {"name": "ui"}],
      "assignees": [{"login": "ian"}],
      "html_url": "https://github.com/example/plexi/issues/285"
    },
    {
      "number": 284,
      "title": "FileList should enforce scoped reads",
      "state": "open",
      "body": "Native ProcessApp FileList must stay inside workspace permissions.",
      "createdAt": "2026-06-22T20:00:00Z",
      "labels": [{"name": "bug"}, {"name": "host"}],
      "assignees": [],
      "html_url": "https://github.com/example/plexi/issues/284"
    },
    {
      "number": 280,
      "title": "Polish canvas app first-frame sizing",
      "state": "open",
      "body": "Canvas apps need pane fallback before canvas feedback arrives.",
      "createdAt": "2026-06-20T11:00:00Z",
      "labels": [{"name": "enhancement"}, {"name": "canvas"}],
      "assignees": [{"login": "agent"}],
      "html_url": "https://github.com/example/plexi/issues/280"
    }
  ],
  "selected": 1,
  "loading": false,
  "pending": "",
  "error": "",
  "filter": "",
  "sort_mode": "created_desc",
  "view": "list",
  "detail": null
}
JSON

  cat > "$out_dir/github-detail-state.json" <<'JSON'
{
  "repo": "plexi/app",
  "issues": [],
  "selected": 0,
  "loading": false,
  "pending": "",
  "error": "",
  "filter": "",
  "sort_mode": "created_desc",
  "view": "detail",
  "detail": {
    "number": 285,
    "title": "SDK v3 native apps visual audit",
    "state": "open",
    "body": "Audit every core app render before merge.\n\nAcceptance:\n- all apps render non-empty\n- GitHub Issues list/detail/loading/error/filter states render\n- game controls remain usable",
    "createdAt": "2026-06-23T15:00:00Z",
    "labels": [{"name": "p1"}, {"name": "sdk-v3"}, {"name": "ui"}],
    "assignees": [{"login": "ian"}],
    "html_url": "https://github.com/example/plexi/issues/285"
  }
}
JSON

  cat > "$out_dir/github-loading-state.json" <<'JSON'
{
  "repo": "plexi/app",
  "issues": [],
  "selected": 0,
  "loading": true,
  "pending": "list",
  "error": "",
  "filter": "",
  "sort_mode": "created_desc",
  "view": "list",
  "detail": null
}
JSON

  cat > "$out_dir/github-error-state.json" <<'JSON'
{
  "repo": "plexi/app",
  "issues": [],
  "selected": 0,
  "loading": false,
  "pending": "",
  "error": "HTTP 403: rate limit exceeded",
  "filter": "",
  "sort_mode": "created_desc",
  "view": "list",
  "detail": null
}
JSON

  cat > "$out_dir/github-filtered-empty-state.json" <<'JSON'
{
  "repo": "plexi/app",
  "issues": [
    {
      "number": 285,
      "title": "SDK v3 native apps visual audit",
      "state": "open",
      "body": "Audit every core app render before merge.",
      "createdAt": "2026-06-23T15:00:00Z",
      "labels": [{"name": "p1"}, {"name": "sdk-v3"}],
      "assignees": [{"login": "ian"}],
      "html_url": "https://github.com/example/plexi/issues/285"
    }
  ],
  "selected": 0,
  "loading": false,
  "pending": "",
  "error": "",
  "filter": "no-such-label",
  "sort_mode": "created_desc",
  "view": "list",
  "detail": null
}
JSON

  cat > "$out_dir/github-filter-active-state.json" <<'JSON'
{
  "repo": "plexi/app",
  "issues": [
    {
      "number": 285,
      "title": "SDK v3 native apps visual audit",
      "state": "open",
      "body": "Audit every core app render before merge.",
      "createdAt": "2026-06-23T15:00:00Z",
      "labels": [{"name": "p1"}, {"name": "sdk-v3"}, {"name": "ui"}],
      "assignees": [{"login": "ian"}],
      "html_url": "https://github.com/example/plexi/issues/285"
    },
    {
      "number": 284,
      "title": "FileList should enforce scoped reads",
      "state": "open",
      "body": "Native ProcessApp FileList must stay inside workspace permissions.",
      "createdAt": "2026-06-22T20:00:00Z",
      "labels": [{"name": "bug"}, {"name": "host"}],
      "assignees": [],
      "html_url": "https://github.com/example/plexi/issues/284"
    },
    {
      "number": 280,
      "title": "Polish canvas app first-frame sizing",
      "state": "open",
      "body": "Canvas apps need pane fallback before canvas feedback arrives.",
      "createdAt": "2026-06-20T11:00:00Z",
      "labels": [{"name": "enhancement"}, {"name": "canvas"}],
      "assignees": [{"login": "agent"}],
      "html_url": "https://github.com/example/plexi/issues/280"
    }
  ],
  "selected": 0,
  "loading": false,
  "pending": "",
  "error": "",
  "filter": "bug",
  "filter_active": true,
  "sort_mode": "created_desc",
  "view": "list",
  "detail": null
}
JSON

  cat > "$out_dir/github-picker-state.json" <<'JSON'
{
  "repo": "plexi/app",
  "issues": [
    {
      "number": 285,
      "title": "SDK v3 native apps visual audit",
      "state": "open",
      "body": "Audit every core app render before merge.",
      "createdAt": "2026-06-23T15:00:00Z",
      "labels": [{"name": "p1"}, {"name": "sdk-v3"}, {"name": "ui"}],
      "assignees": [{"login": "ian"}],
      "html_url": "https://github.com/example/plexi/issues/285"
    },
    {
      "number": 284,
      "title": "FileList should enforce scoped reads",
      "state": "open",
      "body": "Native ProcessApp FileList must stay inside workspace permissions.",
      "createdAt": "2026-06-22T20:00:00Z",
      "labels": [{"name": "bug"}, {"name": "host"}],
      "assignees": [],
      "html_url": "https://github.com/example/plexi/issues/284"
    },
    {
      "number": 280,
      "title": "Polish canvas app first-frame sizing",
      "state": "open",
      "body": "Canvas apps need pane fallback before canvas feedback arrives.",
      "createdAt": "2026-06-20T11:00:00Z",
      "labels": [{"name": "enhancement"}, {"name": "canvas"}],
      "assignees": [{"login": "agent"}],
      "html_url": "https://github.com/example/plexi/issues/280"
    }
  ],
  "selected": 0,
  "loading": false,
  "pending": "",
  "error": "",
  "filter": "",
  "sort_mode": "created_desc",
  "view": "picker",
  "detail": null,
  "picker_query": "",
  "picker_selected": 1,
  "picker_staged": ["bug", "host"]
}
JSON
}

make_contact_sheet() {
  local output="$1"
  shift
  if command -v magick >/dev/null 2>&1; then
    magick montage "$@" -font /System/Library/Fonts/Menlo.ttc -label '%t' \
      -geometry '320x230+12+24>' -tile 4x "$output"
  elif command -v montage >/dev/null 2>&1; then
    montage "$@" -font /System/Library/Fonts/Menlo.ttc -label '%t' \
      -geometry '320x230+12+24>' -tile 4x "$output"
  else
    echo "warning: ImageMagick not found; PNGs rendered but no contact sheet created" >&2
  fi
}

apps=(
  balls
  breakout
  calc
  chess
  csv_viewer
  github-issues
  kraken
  logs
  permissions
  snake
  stats
  sudoku
  tetris
  todo
  wikipedia
)

for app in "${apps[@]}"; do
  render_app "$app" "$repo_root/apps/$app"
done

write_github_state_fixtures
render_app "github-list" "$repo_root/apps/github-issues" "$out_dir/github-list-state.json"
render_app "github-detail" "$repo_root/apps/github-issues" "$out_dir/github-detail-state.json"
render_app "github-loading" "$repo_root/apps/github-issues" "$out_dir/github-loading-state.json"
render_app "github-error" "$repo_root/apps/github-issues" "$out_dir/github-error-state.json"
render_app "github-filtered-empty" "$repo_root/apps/github-issues" "$out_dir/github-filtered-empty-state.json"
render_app "github-filter-active" "$repo_root/apps/github-issues" "$out_dir/github-filter-active-state.json"
render_app "github-picker" "$repo_root/apps/github-issues" "$out_dir/github-picker-state.json"

core_pngs=()
for app in "${apps[@]}"; do
  core_pngs+=("$out_dir/$app.png")
done

state_pngs=(
  "$out_dir/github-list.png"
  "$out_dir/github-detail.png"
  "$out_dir/github-loading.png"
  "$out_dir/github-error.png"
  "$out_dir/github-filtered-empty.png"
  "$out_dir/github-filter-active.png"
  "$out_dir/github-picker.png"
)

make_contact_sheet "$out_dir/core-apps-contact.png" "${core_pngs[@]}"
make_contact_sheet "$out_dir/state-contact.png" "${state_pngs[@]}"
make_contact_sheet "$out_dir/all-app-states-contact.png" "${core_pngs[@]}" "${state_pngs[@]}"

echo "Rendered app audit to $out_dir"
echo "Core sheet: $out_dir/core-apps-contact.png"
echo "State sheet: $out_dir/state-contact.png"
echo "All sheet: $out_dir/all-app-states-contact.png"
