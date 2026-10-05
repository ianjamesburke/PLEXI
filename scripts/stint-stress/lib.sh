# Shared helpers for the stint stress scripts.
# Source this file; do not execute it.
#
# The ledger under test is the stint crate (https://github.com/ianjamesburke/stint),
# pinned below. PLEXI does not vendor it. These scripts never write the caller's
# .stint directory: every case uses a fresh temporary repo.

STINT_REV="${STINT_REV:-d1aadc7665f0ae5ddb2d204b97a340181fc73639}"
STINT_REPO_URL="${STINT_REPO_URL:-https://github.com/ianjamesburke/stint.git}"

section() {
  printf '\n===== %s =====\n' "$*"
}

resolve_stint() {
  if [[ -n "${STINT:-}" && -x "${STINT}" ]]; then
    return 0
  fi
  if [[ -n "${STINT_BIN:-}" && -x "${STINT_BIN}" ]]; then
    STINT="$STINT_BIN"
    return 0
  fi

  local src="${STINT_SRC:-/tmp/stint-src}"
  if [[ ! -d "${src}/.git" ]]; then
    git clone "$STINT_REPO_URL" "$src"
  fi
  local have
  have="$(git -C "$src" rev-parse HEAD)"
  if [[ "$have" != "$STINT_REV" ]]; then
    git -C "$src" fetch --depth 1 origin "$STINT_REV"
    git -C "$src" checkout --detach "$STINT_REV"
  fi

  local cargo_target="${STINT_TARGET_DIR:-/tmp/stint-target}"
  if [[ ! -x "${cargo_target}/release/stint" ]]; then
    (
      cd "$src"
      CARGO_TARGET_DIR="$cargo_target" cargo +stable build --release --locked
    )
  fi
  STINT="${cargo_target}/release/stint"
}

new_repo() {
  local dir
  dir="$(mktemp -d "${TMPDIR:-/tmp}/stint-stress.XXXXXX")"
  (
    cd "$dir"
    "$STINT" init >&2
  )
  printf '%s\n' "$dir"
}

# Print a command, run it, and always continue. Output goes to stdout.
# Usage: run <cmd> [args...]
run() {
  printf '$'
  printf ' %q' "$@"
  printf '\n'
  set +e
  "$@"
  local rc=$?
  set +e
  printf 'exit=%s\n' "$rc"
  return 0
}

# Run a command and print elapsed_sec plus maxrss_kb on stderr after it.
timed() {
  python3 -c '
import resource, subprocess, sys, time
cmd = sys.argv[1:]
start = time.perf_counter()
proc = subprocess.run(cmd)
elapsed = time.perf_counter() - start
rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
print(f"elapsed_sec {elapsed:.3f} max_rss_kb {rss}", file=sys.stderr)
sys.exit(proc.returncode)
' "$@"
}

frontmatter_field() {
  local file="$1"
  local key="$2"
  awk -v key="$key" '
    $0 == "---" { c++; next }
    c == 1 && index($0, key ":") == 1 { print; found=1 }
    END { if (!found) print "(missing " key ")" }
  ' "$file"
}
