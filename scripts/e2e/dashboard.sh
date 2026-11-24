#!/usr/bin/env bash
# Agents dashboard calls from inside a pane (stint 1716 slice 2b).
#
# A process outside every Plexi pane needs a one-use desktop approval for each
# dashboard list, open or follow. These scripts observe the dashboard in loops,
# so they run the verb in a pane of the script's workspace instead, where the
# pane's own context scope covers it. Source this file, then:
#
#   dashboard <agents-dashboard args...>      # prints the verb's stdout, returns its status
#   dashboard_follow <outfile> [errfile]      # streams `--follow` to <outfile>; sets DASHBOARD_FOLLOW_PANE
#
# Requires BIN, BIN_PATH, WORKDIR (the workspace root) and a running host.

dashboard() {
  local tag out err rc pane
  tag="$(mktemp -u "$WORKDIR/dash.XXXXXX")"
  out="$tag.out"; err="$tag.err"; rc="$tag.rc"
  pane="$("$BIN" pane new --cwd "$WORKDIR" --no-focus \
    "\"$BIN_PATH\" agents-dashboard $* >\"$out\" 2>\"$err\"; echo \$? >\"$rc\"" | tr -d '[:space:]')"
  for _ in $(seq 1 100); do
    [[ -s "$rc" ]] && break
    sleep 0.1
  done
  "$BIN" pane close "$pane" >/dev/null 2>&1 || true
  if [[ ! -s "$rc" ]]; then
    echo "dashboard: no result for 'agents-dashboard $*' from pane $pane" >&2
    return 1
  fi
  cat "$out"
  cat "$err" >&2
  return "$(cat "$rc")"
}

dashboard_follow() {
  local out="$1" err="${2:-/dev/null}"
  DASHBOARD_FOLLOW_PANE="$("$BIN" pane new --cwd "$WORKDIR" --no-focus \
    "\"$BIN_PATH\" agents-dashboard --follow >\"$out\" 2>\"$err\"" | tr -d '[:space:]')"
}
