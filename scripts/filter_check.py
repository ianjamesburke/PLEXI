#!/usr/bin/env python3
"""Fail fast when a libtest filter matches no test, and say what was meant.

libtest exits 0 when a filter selects nothing, so a mistyped filter reads as a
pass. Every recipe that takes a filter goes through here instead:

  host -- ARGS...        `just test-host`: validate, then run the host tests.
  for PATH               `just test-for`: tests covering one source file.
  precheck -- FILTERS    validate against the cached test list, no build.
  key                    print the source key (take it before listing; pass to validate --key-before).
  validate [-] -- FILTERS  validate a full `--list` output (file or stdin).
  suite -- SKIPS...      `just test`: run every test target except SKIPS (libtest
                         substring skips) and report how many tests that skipped.

Test list cache. The list of test names depends only on the package's Rust
sources and manifests, so it is keyed on a SHA-256 of those files' bytes
(`source_key`). A cache entry is served as truth only for the exact key it was
written under; any edit to a source file, Cargo.toml or Cargo.lock changes the
key and the entry is then ignored (reported as stale) and the filters are
validated after the build from a fresh `--list`. Tests defined through
`include!` of a non-.rs file are the one input the key does not see. The
post-run guard (`running 0 tests` with exit 0 is an error) backstops that.
"""

import argparse
import hashlib
import json
import os
import platform
import subprocess
import sys
import time
from pathlib import Path

CACHE_ENV = "PLEXI_TEST_LIST_CACHE_DIR"
CACHE_KEEP = 20
# Options that consume the next argv token, so it is not a filter.
VALUE_OPTIONS = {"--skip", "--test-threads", "--format", "--color", "--logfile",
                 "--shuffle-seed", "-Z"}
# Options that make validation meaningless: nothing is filtered or run.
BYPASS_OPTIONS = {"--list", "--help", "-h", "--version"}
NOKEY = object()  # "compute the key now"; None is a real value: no src/ tree to key on
MANIFESTS = ("Cargo.toml", "Cargo.lock", "build.rs", "rust-toolchain", "rust-toolchain.toml",
             ".cargo/config.toml")


def exit_code(returncode):
    """Shell convention for a signalled child: 128+signal, as cargo-with-lease.sh reports it."""
    return 128 + -returncode if returncode < 0 else returncode


def say(message):
    print(f"filter-check: {message}", file=sys.stderr, flush=True)


# --- argument and list handling -------------------------------------------

def parse_args(args):
    """Split libtest args into filters, skips and the flags that change matching."""
    parsed = {"filters": [], "skips": [], "exact": False, "ignored": False, "bypass": False}
    tokens = list(args)
    index = 0
    while index < len(tokens):
        token = tokens[index]
        index += 1
        if token in BYPASS_OPTIONS:
            parsed["bypass"] = True
        elif token == "--exact":
            parsed["exact"] = True
        elif token == "--ignored":
            parsed["ignored"] = True
        elif token == "--skip":
            if index < len(tokens):
                parsed["skips"].append(tokens[index])
                index += 1
        elif token.startswith("--skip="):
            parsed["skips"].append(token.split("=", 1)[1])
        elif token in VALUE_OPTIONS:
            index += 1
        elif token.startswith("-") and token != "-":
            continue
        else:
            parsed["filters"].append(token)
    return parsed


def parse_listing(text):
    """Test names from `--list` output: lines of the form `path::to::test: test`."""
    return [line[:-len(": test")] for line in text.splitlines() if line.endswith(": test")]


def matches(name, pattern, exact):
    return name == pattern if exact else pattern in name


def evaluate(names, filters, skips, exact):
    """Per-filter match counts, plus the tests that would actually run."""
    per_filter = {f: sum(1 for n in names if matches(n, f, exact)) for f in filters}
    selected = [n for n in names
                if (not filters or any(matches(n, f, exact) for f in filters))
                and not any(matches(n, s, exact) for s in skips)]
    return per_filter, selected


# --- suggestions ----------------------------------------------------------

def levenshtein(a, b):
    previous = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        current = [i]
        for j, cb in enumerate(b, 1):
            current.append(min(previous[j] + 1, current[j - 1] + 1, previous[j - 1] + (ca != cb)))
        previous = current
    return previous[-1]


def module_paths(names):
    found = set()
    for name in names:
        parts = name.split("::")
        for end in range(1, len(parts)):
            found.add("::".join(parts[:end]) + "::")
    return sorted(found)


def nearest(pool, wanted, limit):
    """Rank candidates: shared word fragments first, then edit distance on path segments."""
    wanted = wanted.lower()
    tokens = [t for t in wanted.replace("::", "_").split("_") if len(t) >= 3]
    scored = []
    for candidate in pool:
        low = candidate.lower()
        hits = sum(1 for t in tokens if t in low)
        if hits:
            segments = [s for s in low.rstrip(":").split("::") if s] + [low]
            scored.append((-hits, min(levenshtein(wanted, s) for s in segments), len(candidate), candidate))
    if not scored:
        for candidate in pool:
            low = candidate.lower()
            scored.append((0, levenshtein(wanted, low.rstrip(":").split("::")[-1]), len(candidate), candidate))
    return [item[-1] for item in sorted(scored)[:limit]]


def report_zero(label, names, filter_, origin):
    say(f"{label} filter matched zero tests: {filter_}")
    if not names:
        say("  the test binary lists no tests at all")
    else:
        modules = nearest(module_paths(names), filter_, 4)
        tests = nearest(names, filter_, 4)
        if modules:
            say("  nearest modules: " + ", ".join(modules))
        if tests:
            say("  nearest tests:")
            for test in tests:
                say(f"    {test}")
        say("  a filter is a substring of the full path (`module::submodule::test`); use `::` separators")
    say(f"  ({origin})")


# --- cache ----------------------------------------------------------------

def source_key(root):
    """Content hash of everything that decides which tests exist, or None with no src/."""
    root = Path(root)
    source = root / "src"
    if not source.is_dir():
        return None
    digest = hashlib.sha256()
    digest.update(f"{sys.platform}/{platform.machine()}\n".encode())
    files = sorted(p for p in source.rglob("*.rs") if p.is_file())
    files += [root / name for name in MANIFESTS if (root / name).is_file()]
    for path in files:
        digest.update(str(path.relative_to(root)).encode() + b"\0")
        digest.update(path.read_bytes() + b"\0")
    return digest.hexdigest()


def cache_dir():
    explicit = os.environ.get(CACHE_ENV)
    if explicit:
        return Path(explicit)
    base = os.environ.get("XDG_CACHE_HOME") or str(Path.home() / ".cache")
    return Path(base) / "plexi-test-lists"


def cache_load(key):
    if key is None:
        return None
    try:
        record = json.loads((cache_dir() / f"{key}.json").read_text())
    except (OSError, ValueError):
        return None
    return record if record.get("key") == key and isinstance(record.get("names"), list) else None


def cache_store(key, names):
    if key is None:
        return
    directory = cache_dir()
    try:
        directory.mkdir(parents=True, exist_ok=True)
        temp = directory / f"{key}.json.{os.getpid()}.tmp"
        temp.write_text(json.dumps({"key": key, "listed_at": time.time(), "names": names}))
        os.replace(temp, directory / f"{key}.json")
        entries = sorted(directory.glob("*.json"), key=lambda p: p.stat().st_mtime, reverse=True)
        for old in entries[CACHE_KEEP:]:
            old.unlink(missing_ok=True)
    except OSError as error:
        say(f"warning: could not write the test list cache at {directory}: {error}")


# --- validation -----------------------------------------------------------

def judge(names, parsed, label, origin):
    """Print errors; return the number of tests selected, or None on failure."""
    filters = parsed["filters"]
    per_filter, selected = evaluate(names, filters, parsed["skips"], parsed["exact"])
    zero = [f for f, count in per_filter.items() if count == 0]
    if filters and len(zero) == len(filters):
        say(f"{label} filters matched zero tests")
    for f in zero:
        report_zero(label, names, f, origin)
    if zero:
        return None
    if not selected:
        say(f"{label} filters matched tests, but --skip excluded every one of them")
        return None
    return len(selected)


def precheck(root, parsed, label):
    """Validate against the cached list. Returns 'ok', 'zero' or 'unknown'."""
    key = source_key(root)
    if key is None:
        say("no src/ tree to key a test list on; filters are validated after the listing")
        return "unknown"
    record = cache_load(key)
    if record is None:
        say(f"no cached test list for this source state (key {key[:12]}): any earlier list is stale "
            "or none exists; filters are validated after the test build instead")
        return "unknown"
    origin = f"cached test list for this exact source state, key {key[:12]}, " \
             f"listed {time.strftime('%Y-%m-%d %H:%M:%S', time.localtime(record.get('listed_at', 0)))}"
    count = judge(record["names"], parsed, label, origin)
    if count is None:
        return "zero"
    say(f"filters select {count} tests ({origin})")
    return "ok"


def validate_listing(root, parsed, label, text, store, key_before=NOKEY):
    """key_before is the source key taken BEFORE the listing was produced; the list is cached
    only if the source is unchanged since, so an edit during the build never poisons the cache."""
    names = parse_listing(text)
    key = source_key(root) if key_before is NOKEY else key_before
    if store:
        if key == source_key(root):
            cache_store(key, names)
        else:
            say("source changed while the tests were being built and listed; not caching this list")
    origin = f"fresh test listing of {len(names)} tests" + (f", source key {key[:12]}" if key else "")
    count = judge(names, parsed, label, origin)
    return count


# --- running --------------------------------------------------------------

def cargo_base(root):
    return ["python3", str(Path(root) / "scripts/cargo-channel-inputs.py"), "test", "--bin", "plexi", "--"]


def locked_run(root, args):
    """Runs under the cargo lease taken by `host`: list if needed, validate, run, guard."""
    parsed = parse_args(args)
    root = Path(root)
    if parsed["filters"] and not parsed["bypass"]:
        key_before = source_key(root)
        record = None if parsed["ignored"] else cache_load(key_before)
        if record is None:
            listing = subprocess.run(cargo_base(root) + (["--ignored"] if parsed["ignored"] else []) + ["--list"],
                                     cwd=root, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                     text=True, errors="replace")
            if listing.returncode != 0:
                say(f"listing the tests failed (exit {exit_code(listing.returncode)}); see the build output above")
                return exit_code(listing.returncode)
            if validate_listing(root, parsed, "test-host", listing.stdout, store=not parsed["ignored"],
                                key_before=key_before) is None:
                return 1
    process = subprocess.Popen(cargo_base(root) + list(args), cwd=root, stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, text=True, errors="replace", bufsize=1)
    last_count = None
    for line in process.stdout:
        sys.stdout.write(line)
        sys.stdout.flush()
        if line.startswith("running ") and " test" in line:
            try:
                last_count = int(line.split()[1])
            except ValueError:
                pass
    code = exit_code(process.wait())
    if code == 0 and parsed["filters"] and not parsed["bypass"] and last_count == 0:
        say("the run executed zero tests although filters were given (the validated list was wrong or "
            "stale); treating this as a failure, not a pass")
        return 1
    return code


def run_host(root, args):
    parsed = parse_args(args)
    root = Path(root)
    if not parsed["filters"] or parsed["bypass"]:
        return exit_code(subprocess.call(["bash", str(root / "scripts/cargo-with-lease.sh")] + cargo_base(root)
                                         + list(args), cwd=root))
    if not parsed["ignored"] and precheck(root, parsed, "test-host") == "zero":
        return 1
    return exit_code(subprocess.call(["bash", str(root / "scripts/cargo-with-lease.sh"), "python3",
                                      str(Path(__file__).resolve()), "locked-run", "--root", str(root), "--", *args],
                                     cwd=root))


def skip_summary(names, skips):
    """One line: total skipped, per-pattern counts, and how many matched only the function name."""
    per = {skip: sum(1 for n in names if skip in n) for skip in skips}
    skipped = [n for n in names if any(skip in n for skip in skips)]
    name_only = [n for n in skipped if not any(skip in n.rsplit("::", 1)[0] for skip in skips)]
    return (f"just test is NOT the full suite: skipped {len(skipped)} of {len(names)} tests "
            f"({len(name_only)} of them only because a pattern occurs in the test function's name, "
            f"not its module path). Patterns are substrings: "
            + ", ".join(f"{skip}={count}" for skip, count in per.items())
            + ". `just release-host-gate` runs everything.")


def locked_suite(root, skips):
    """Under the cargo lease: list every target, announce the skips, run, announce them again."""
    base = ["python3", str(Path(root) / "scripts/cargo-channel-inputs.py"), "test", "--"]
    listing = subprocess.run(base + ["--list"], cwd=root, stdin=subprocess.DEVNULL,
                             stdout=subprocess.PIPE, text=True, errors="replace")
    if listing.returncode != 0:
        say(f"listing the tests failed (exit {exit_code(listing.returncode)}); see the build output above")
        return exit_code(listing.returncode)
    summary = skip_summary(parse_listing(listing.stdout), skips)
    say(summary)
    arguments = [part for skip in skips for part in ("--skip", skip)]
    code = exit_code(subprocess.call(base + arguments, cwd=root, stdin=subprocess.DEVNULL))
    say(summary)
    return code


def run_suite(root, skips):
    return exit_code(subprocess.call(["bash", str(Path(root) / "scripts/cargo-with-lease.sh"), "python3",
                                      str(Path(__file__).resolve()), "locked-suite", "--root", str(root), "--", *skips],
                                     cwd=root))


def scoped_filters(root, relative):
    result = subprocess.run(["bash", str(Path(root) / "scripts/scoped-tests.sh"), relative],
                            cwd=root, capture_output=True, text=True, errors="replace")
    if result.returncode != 0:
        say(f"scripts/scoped-tests.sh failed for {relative}: {result.stderr.strip()}")
        return None
    return [line for line in result.stdout.splitlines() if line]


def module_filter(relative):
    """`src/host/model.rs` -> `host::model::`; None for a crate-root file."""
    parts = Path(relative).with_suffix("").parts[1:]
    if parts and parts[-1] == "mod":
        parts = parts[:-1]
    if not parts or parts in (("main",), ("lib",)):
        return None
    return "::".join(parts) + "::"


NO_HOST_TESTS = (("scripts/", "run `just test-infra-scripts`"),
                 ("apps/", "run `just sdk-smoke` and the app's own checks"),
                 ("sdk/python/", "run `just sdk-smoke`"))


def run_for(root, path):
    root = Path(root)
    absolute = (Path.cwd() / path).resolve()
    if not absolute.is_file():
        say(f"test-for: no such file: {path}")
        return 2
    try:
        relative = absolute.relative_to(root.resolve()).as_posix()
    except ValueError:
        say(f"test-for: {path} is outside the repository {root}")
        return 2
    filters = scoped_filters(root, relative)
    if filters is None:
        return 1
    if not filters:
        hint = next((h for prefix, h in NO_HOST_TESTS if relative.startswith(prefix)), "no host test applies")
        say(f"test-for: {relative} selects no host Rust tests (path table entry is empty); {hint}")
        return 0
    if filters == ["::"]:
        fallback = module_filter(relative) if relative.endswith(".rs") and relative.startswith("src/") else None
        if fallback is None:
            say(f"test-for: {relative} has no entry in scripts/scoped-tests.table and no module of its own; "
                "pick filters by hand: just test-host <filter>")
            return 2
        say(f"test-for: FALLBACK: no entry in scripts/scoped-tests.table covers {relative}; "
            f"running its own module tests only ({fallback}). Dependents are not covered.")
        filters = [fallback]
    else:
        say(f"test-for: {relative} -> scripts/scoped-tests.table filters: {' '.join(filters)}")
    return run_host(root, filters)


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", default=str(Path(__file__).resolve().parents[1]))
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("host", "locked-run", "locked-suite", "suite", "precheck", "validate"):
        command = sub.add_parser(name)
        command.add_argument("--root", dest="sub_root")
        command.add_argument("--label", default="test filter")
        if name == "validate":
            command.add_argument("listing", nargs="?", default="-")
            command.add_argument("--no-store", action="store_true")
            command.add_argument("--key-before", default=None,
                                 help="output of `key`, taken before the listing was produced")
        command.add_argument("args", nargs=argparse.REMAINDER)
    sub.add_parser("key").add_argument("--root", dest="sub_root")
    command = sub.add_parser("for")
    command.add_argument("path")
    options = parser.parse_args(argv)
    root = getattr(options, "sub_root", None) or options.root
    args = list(getattr(options, "args", []) or [])
    if args[:1] == ["--"]:
        args = args[1:]
    if options.command == "host":
        return run_host(root, args)
    if options.command == "locked-run":
        return locked_run(root, args)
    if options.command == "suite":
        return run_suite(root, args)
    if options.command == "locked-suite":
        return locked_suite(root, args)
    if options.command == "key":
        print(source_key(root) or "")
        return 0
    if options.command == "for":
        return run_for(root, options.path)
    parsed = parse_args(args)
    parsed["filters"] = [a for a in args]  # filters given verbatim, never option-parsed
    parsed["skips"], parsed["exact"] = [], False
    if options.command == "precheck":
        return 1 if precheck(root, parsed, options.label) == "zero" else 0
    text = sys.stdin.read() if options.listing == "-" else Path(options.listing).read_text()
    key_before = NOKEY if options.key_before is None else (options.key_before or None)
    count = validate_listing(root, parsed, options.label, text, store=not options.no_store, key_before=key_before)
    if count is None:
        return 1
    print(f"matched={count}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
