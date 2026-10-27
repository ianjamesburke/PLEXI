#!/usr/bin/env bash
# Usage: pr-install-pretests.sh <worktree> <base-oid> <scoped|full> <head-ref> <base-ref>
# PLEXI_PR_TEST_FILTERS supplies newline-separated reviewed filters when automatic
# selection is too broad. Empty selection from the selector means SDK checks only.
set -euo pipefail
worktree="${1:?worktree required}"
base_oid="${2:?base oid required}"
scope="${3:-scoped}"
head_ref="${4:?head ref required}"
base_ref="${5:?base ref required}"
cd "$worktree"
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
  bash scripts/cargo-with-lease.sh env -u PLEXI_CHANNEL -u PLEXI_CONTEXT_ROOT \
    -u PLEXI_CONTEXT_ID -u PLEXI_CONTEXT_NAME -u PLEXI_SOCKET -u PLEXI_RUNNING \
    -u PLEXI_PANE_ID cargo test --bin plexi -- "$@" < /dev/null
}
if [[ "$scope" == scoped && ${#filters[@]} -gt 0 ]]; then
  listing="$(cargo_test "${filters[@]}" --list)"
  matched="$(printf '%s\n' "$listing" | awk '/: test$/ {n++} END {print n+0}')"
  if [[ "$matched" == 0 ]]; then
    echo "pr-install: scoped Rust filters matched zero tests" >&2
    exit 1
  fi
  for filter in "${filters[@]}"; do
    count="$(printf '%s\n' "$listing" | awk -v filter="$filter" '/: test$/ && index($0, filter) {n++} END {print n+0}')"
    if [[ "$count" == 0 ]]; then
      echo "pr-install: scoped Rust filter matched zero tests: $filter" >&2
      exit 1
    fi
  done
  printf 'pr-install: scoped Rust filters=%s matched=%s base=%s\n' "${filters[*]}" "$matched" "$base_oid"
fi
just fetch-python-runtime sdk-smoke
if [[ "$scope" == full ]]; then
  echo "pr-install: full final-candidate pretests base=$base_oid"
  cargo_test
elif [[ ${#filters[@]} -gt 0 ]]; then
  cargo_test "${filters[@]}"
else
  echo "pr-install: scoped pretests select no host Rust tests; SDK and affected-script checks remain required"
fi
