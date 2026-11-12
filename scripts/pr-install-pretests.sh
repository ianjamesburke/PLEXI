#!/usr/bin/env bash
# Usage: pr-install-pretests.sh <worktree> <base-oid> <scoped|full> <head-ref> <base-ref>
# PLEXI_PR_TEST_FILTERS supplies newline-separated reviewed filters when automatic
# selection is too broad. Empty selection from the selector means SDK checks only.
set -euo pipefail
driver_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
worktree="${1:?worktree required}"
base_oid="${2:?base oid required}"
scope="${3:-scoped}"
head_ref="${4:?head ref required}"
base_ref="${5:?base ref required}"
cd "$worktree"
if [[ -z "${PLEXI_PR_REQUIRED_PTY_FILTERS:-}" || -z "${PLEXI_PR_REQUIRED_SCENES:-}" ]]; then
  echo "pr-install: declare both required PTY filters and scenes (newline-separated selections or 'none')" >&2
  exit 1
fi
for selection in "$PLEXI_PR_REQUIRED_PTY_FILTERS" "$PLEXI_PR_REQUIRED_SCENES"; do
  if [[ "$selection" != none ]] && printf '%s\n' "$selection" | grep -qx none; then
    echo "pr-install: 'none' must be the entire selection" >&2; exit 1
  fi
done
case "$scope" in
  scoped) ;;
  full)
    expected="$(uv run --no-project python -c 'import tomllib; c=tomllib.load(open(".agents/skills/babysitter-v3/RUN_CONFIG.toml", "rb"))["topology"]; print(c["branch"] + " " + c["final_base"])')"
    if [[ "$head_ref $base_ref" != "$expected" ]]; then
      echo "pr-install: full tests are reserved for the final combined candidate" >&2
      exit 1
    fi
    ;;
  *) echo "pr-install: test scope must be scoped or full" >&2; exit 1 ;;
esac
filters=()
if [[ "$scope" == scoped ]]; then
  if [[ "${PLEXI_PR_TEST_FILTERS+x}" == x ]]; then
    selection="$PLEXI_PR_TEST_FILTERS"
    [[ -n "$selection" ]] || { echo "pr-install: empty explicit filters are forbidden" >&2; exit 1; }
  else
    selection="$(SCOPED_TEST_BASE="$base_oid" bash scripts/scoped-tests.sh)"
  fi
  while IFS= read -r filter; do
    [[ -n "$filter" ]] || continue
    if [[ "$filter" == :: || "$filter" == -* || ! "$filter" =~ [[:alnum:]_] ]]; then
      echo "pr-install: broad selection requires reviewed PLEXI_PR_TEST_FILTERS; full tests belong to the final candidate" >&2
      exit 1
    fi
    filters+=("$filter")
  done <<< "$selection"
fi
cargo_test() {
  # Older PR worktrees may predate the managed input helper.
  local cargo_command=(cargo)
  if [[ -f scripts/cargo-channel-inputs.py ]]; then
    cargo_command=(python3 scripts/cargo-channel-inputs.py)
  fi
  bash scripts/cargo-with-lease.sh env -u PLEXI_CHANNEL -u PLEXI_CONTEXT_ROOT \
    -u PLEXI_CONTEXT_ID -u PLEXI_CONTEXT_NAME -u PLEXI_SOCKET -u PLEXI_RUNNING \
    -u PLEXI_PANE_ID "${cargo_command[@]}" test --bin plexi -- "$@" < /dev/null
}
filter_check=(python3 "$driver_dir/filter_check.py" --root "$worktree")
if [[ "$scope" == scoped && ${#filters[@]} -gt 0 ]]; then
  # Cheapest first: a cached list for this exact source state rejects a wrong
  # filter before any compile. Otherwise validate right after the test build.
  "${filter_check[@]}" precheck --label "pr-install: scoped Rust" -- "${filters[@]}"
  key_before="$("${filter_check[@]}" key)"
  listing="$(cargo_test --list)"
  matched="$(printf '%s\n' "$listing" | "${filter_check[@]}" validate --label "pr-install: scoped Rust" --key-before "$key_before" - -- "${filters[@]}" | sed -n 's/^matched=//p')"
  printf 'pr-install: scoped Rust filters=%s matched=%s base=%s\n' "${filters[*]}" "$matched" "$base_oid"
fi
sdk_selection="run: full candidate or legacy selector"
if [[ "$scope" == scoped && -f "$driver_dir/pr-sdk-selection.py" ]]; then
  sdk_selection="$(python3 "$driver_dir/pr-sdk-selection.py" "$base_oid")"
fi
if [[ "${PLEXI_PR_REQUIRE_SDK:-}" == 1 ]]; then
  sdk_selection="run: explicit qualification"
fi
echo "pr-install: SDK smoke $sdk_selection"
if [[ "$sdk_selection" != skip:* ]]; then
  just sdk-smoke
fi
if [[ "$scope" == full ]]; then
  echo "pr-install: full final-candidate pretests base=$base_oid"
  cargo_test
elif [[ ${#filters[@]} -gt 0 ]]; then
  cargo_test "${filters[@]}"
else
  echo "pr-install: scoped pretests select no host Rust tests; affected-script checks remain required; SDK decision recorded above"
fi

# Required opt-in PTY regressions are explicit; never run all ignored tests.
while IFS= read -r filter; do
  [[ -n "$filter" ]] || continue
  [[ "$filter" != none ]] || continue
  if [[ "$filter" == :: || "$filter" == -* || ! "$filter" =~ [[:alnum:]_] ]]; then
    echo "pr-install: invalid required PTY filter: $filter" >&2; exit 1
  fi
  listing="$(cargo_test "$filter" --ignored --list)"
  count="$(printf '%s\n' "$listing" | awk -v filter="$filter" '/: test$/ && index($0, filter) {n++} END {print n+0}')"
  [[ "$count" -gt 0 ]] || { echo "pr-install: required PTY filter matched zero tests: $filter" >&2; exit 1; }
  echo "pr-install: required PTY filter=$filter matched=$count"
  cargo_test "$filter" --ignored
done <<< "${PLEXI_PR_REQUIRED_PTY_FILTERS:-}"

# Scene paths are tracked inputs from the selected clean head, not arbitrary commands.
while IFS= read -r scene; do
  [[ -n "$scene" ]] || continue
  [[ "$scene" != none ]] || continue
  if [[ "$scene" != tests/scenes/*.toml || "$scene" == *..* || -L "$scene" ]] || ! git ls-files --error-unmatch -- "$scene" >/dev/null 2>&1; then
    echo "pr-install: invalid or untracked required scene: $scene" >&2; exit 1
  fi
  echo "pr-install: required scene=$scene"
  just scene "$scene"
done <<< "${PLEXI_PR_REQUIRED_SCENES:-}"
