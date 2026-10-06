#!/usr/bin/env bash
# Human-intent test driver (W15).
#
# Real OS pointer events on the X display. Source this file, then:
#
#   HUMAN_APPROVE <pending_id> [once|session|always]
#   HUMAN_DENY <pending_id>
#   HUMAN_PLAY_UCI <uci>          # click the chess board (e2e4, g1f3, …)
#
# Requires BIN (the installed channel binary), DISPLAY, and xdotool.
# Board clicks also require Pillow on the python3 that runs board_click.py.
# Button rects come from `pane state` (accesskit bounds) or, when the host
# publishes them, from `assistant permission list` → buttons. The click is
# xdotool on the host window — never pane click, pane key, or a resolve CLI.

human__root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Board squares are found by scripts/e2e/board_click.py, which imports PIL.
# A missing module used to surface as "could not locate" after the click was
# skipped. Callers should exit on failure instead of continuing the suite.
human__require_pillow() {
  local py err
  py="$(command -v python3 || true)"
  if [[ -z "$py" ]]; then
    echo "human: env error: board clicks need python3 with Pillow, and python3 is not on PATH" >&2
    return 1
  fi
  if err="$("$py" -c 'import PIL' 2>&1)"; then
    return 0
  fi
  echo "human: env error: board clicks need Pillow for $py ($err)" >&2
  return 1
}

human__label_for() {
  case "$1" in
    once|approve) printf '%s' "Allow once" ;;
    session) printf '%s' "Allow this session" ;;
    always) printf '%s' "Always allow" ;;
    deny) printf '%s' "Deny" ;;
    *) printf '%s' "$1" ;;
  esac
}

human__host_pid() {
  "$BIN" host status --json 2>/dev/null | python3 -c 'import json,sys
try:
    data=json.load(sys.stdin)
except Exception:
    print(""); raise SystemExit
print(data.get("pid") or "")'
}

human__window_id() {
  local pid="$1"
  local wid=""
  if [[ -n "$pid" ]]; then
    wid="$(xdotool search --pid "$pid" 2>/dev/null | head -1 || true)"
  fi
  if [[ -z "$wid" ]]; then
    wid="$(xdotool search --class plexi 2>/dev/null | head -1 || true)"
  fi
  if [[ -z "$wid" ]]; then
    wid="$(xdotool search --name "Plexi" 2>/dev/null | head -1 || true)"
  fi
  printf '%s' "$wid"
}

human__pane_ids() {
  "$BIN" pane list 2>/dev/null | python3 -c 'import json,sys
try:
    rows=json.load(sys.stdin)
except Exception:
    raise SystemExit
for row in rows:
    if row.get("type") == "app":
        print(row.get("id",""))'
}

# Print "x y" window-relative center for a button label, or nothing.
human__button_center() {
  local label="$1"
  local tmp
  tmp="$(mktemp)"
  local pane
  while read -r pane; do
    [[ -z "$pane" ]] && continue
    "$BIN" pane state "$pane" >"$tmp" 2>/dev/null || continue
    local hit
    hit="$(python3 - "$tmp" "$label" <<'PY'
import json, sys
path, label = sys.argv[1:]
try:
    data = json.load(open(path))
except Exception:
    raise SystemExit
nodes = (data.get("semantic") or {}).get("nodes") or data.get("nodes") or []
for node in nodes:
    if str(node.get("role","")).lower() != "button":
        continue
    if (node.get("label") or "") != label:
        continue
    bounds = node.get("bounds")
    if not bounds or len(bounds) != 4:
        continue
    x0, y0, x1, y1 = bounds
    print(f"{(x0+x1)/2:.1f} {(y0+y1)/2:.1f}")
    raise SystemExit
PY
)"
    if [[ -n "$hit" ]]; then
      rm -f "$tmp"
      printf '%s' "$hit"
      return 0
    fi
  done < <(human__pane_ids)
  # Host-published banner buttons (W2). Absent on builds that only have the sheet.
  "$BIN" assistant permission list >"$tmp" 2>/dev/null || true
  local hit
  hit="$(python3 - "$tmp" "$label" <<'PY'
import json, sys
path, label = sys.argv[1:]
try:
    data = json.load(open(path))
except Exception:
    raise SystemExit
for button in data.get("buttons") or []:
    if (button.get("label") or "") != label:
        continue
    bounds = button.get("bounds")
    if not bounds or len(bounds) != 4:
        continue
    x0, y0, x1, y1 = bounds
    print(f"{(x0+x1)/2:.1f} {(y0+y1)/2:.1f}")
    raise SystemExit
PY
)"
  rm -f "$tmp"
  if [[ -n "$hit" ]]; then
    printf '%s' "$hit"
    return 0
  fi
  return 1
}

human__pending_present() {
  local id="$1"
  local tmp
  tmp="$(mktemp)"
  "$BIN" assistant permission list >"$tmp" 2>/dev/null || true
  python3 - "$tmp" "$id" <<'PY'
import json, sys
path, want = sys.argv[1:]
try:
    data = json.load(open(path))
except Exception:
    raise SystemExit(1)
rows = data.get("pending") or []
ids = []
for row in rows:
    if isinstance(row, dict):
        ids.append(str(row.get("pending_request_id") or row.get("id") or ""))
if want in ids:
    raise SystemExit(0)
raise SystemExit(1)
PY
  local code=$?
  rm -f "$tmp"
  return "$code"
}

# Screen origin of the window's client area. xdotool getwindowgeometry
# reports X/Y that include the reparented frame offset a second time, so a
# click aimed at an accesskit rect lands below the button.
human__window_origin() {
  local wid="$1"
  local origin=""
  if command -v xwininfo >/dev/null 2>&1; then
    origin="$(xwininfo -id "$wid" 2>/dev/null | awk '
      /Absolute upper-left X:/ { x = $NF }
      /Absolute upper-left Y:/ { y = $NF }
      END { if (x != "" && y != "") print x, y }
    ')"
  fi
  if [[ -n "$origin" ]]; then
    printf '%s' "$origin"
    return 0
  fi
  local geo
  geo="$(xdotool getwindowgeometry --shell "$wid" 2>/dev/null || true)"
  # shellcheck disable=SC2086
  eval "$geo"
  printf '%s %s' "${X:-0}" "${Y:-0}"
}

human__click_window() {
  local wid="$1" x="$2" y="$3" _mode="${4:-screen}"
  xdotool windowactivate --sync "$wid" >/dev/null 2>&1 || true
  local origin ox oy sx sy
  origin="$(human__window_origin "$wid")"
  ox="${origin%% *}"
  oy="${origin##* }"
  sx="$(python3 -c "print(int(float('$ox') + float('$x')))")"
  sy="$(python3 -c "print(int(float('$oy') + float('$y')))")"
  # XTEST pointer events. `click --window` is XSendEvent, which winit drops,
  # so the press never reaches the permission sheet.
  xdotool mousemove --sync "$sx" "$sy"
  xdotool click 1
}

# Click a labeled approval button with a real pointer event.
# Returns 0 when pending_id is no longer listed.
human__click_choice() {
  local pending_id="$1" choice="$2"
  local label
  label="$(human__label_for "$choice")"
  local pid wid center
  pid="$(human__host_pid)"
  wid="$(human__window_id "$pid")"
  if [[ -z "$wid" ]]; then
    echo "human: no host window for pid '$pid' on DISPLAY=$DISPLAY" >&2
    return 1
  fi
  local attempt mode
  for attempt in 1 2 3 4 5 6 7 8; do
    center="$(human__button_center "$label" || true)"
    if [[ -z "$center" ]]; then
      sleep 0.4
      continue
    fi
    local x y
    x="${center%% *}"
    y="${center##* }"
    mode="xtest"
    echo "human: click '$label' at ${x},${y} mode=$mode window=$wid attempt=$attempt" >&2
    human__click_window "$wid" "$x" "$y" "$mode" || true
    sleep 0.6
    if ! human__pending_present "$pending_id"; then
      echo "human: $choice resolved $pending_id" >&2
      return 0
    fi
  done
  echo "human: '$label' click did not resolve $pending_id" >&2
  return 1
}

HUMAN_APPROVE() {
  local id="${1:?pending id}"
  local choice="${2:-once}"
  human__click_choice "$id" "$choice"
}

HUMAN_DENY() {
  local id="${1:?pending id}"
  human__click_choice "$id" "deny"
}

# Click two chess squares so the local human plays a UCI move (e2e4).
# Uses a full-window screenshot to find the board, then xdotool.
HUMAN_PLAY_UCI() {
  local uci="${1:?uci move}"
  if ! human__require_pillow; then
    exit 1
  fi
  local pid wid shot
  pid="$(human__host_pid)"
  wid="$(human__window_id "$pid")"
  if [[ -z "$wid" ]]; then
    echo "human: no host window for a board click" >&2
    return 1
  fi
  shot="$(mktemp --suffix=.png)"
  if ! "$BIN" host screenshot --output "$shot" >/dev/null 2>&1; then
    echo "human: host screenshot failed" >&2
    rm -f "$shot"
    return 1
  fi
  local coords
  coords="$(python3 "$human__root/board_click.py" "$shot" "$uci")" || {
    echo "human: could not locate $uci on the chess board" >&2
    rm -f "$shot"
    return 1
  }
  rm -f "$shot"
  local x1 y1 x2 y2
  read -r x1 y1 x2 y2 <<<"$coords"
  echo "human: board click $uci ($x1,$y1) -> ($x2,$y2) window=$wid" >&2
  human__click_window "$wid" "$x1" "$y1" "window" || return 1
  sleep 0.45
  human__click_window "$wid" "$x2" "$y2" "window" || return 1
  sleep 0.4
}
