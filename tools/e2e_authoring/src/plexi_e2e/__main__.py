"""``plexi-e2e`` — run one agent-drives-agent app-authoring session from a fixture.

    plexi-e2e run <fixture.toml> [--channel e2e] [--dry-run] [--fresh-profile]

Provisions an isolated session, executes the fixture, and leaves a complete
session directory under the sessions root. ``--dry-run`` records the exact plan
without booting a host — use it to validate plumbing anywhere.
"""

from __future__ import annotations

import argparse
import logging
import sys
from pathlib import Path

from .config import Fixture, SessionConfig, default_binary_for
from .runner import E2ESession

DEFAULT_SESSIONS_ROOT = Path("benchmarks/app-authoring/sessions")


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="plexi-e2e", description=__doc__)
    sub = p.add_subparsers(dest="cmd", required=True)

    run = sub.add_parser("run", help="run one session from a fixture")
    run.add_argument("fixture", type=Path, help="path to a prompt fixture TOML")
    run.add_argument("--channel", default="e2e", help="host channel to drive (default: e2e)")
    run.add_argument("--binary", default=None, help="CLI binary name (default: plexi-<channel>)")
    run.add_argument(
        "--sessions-root", type=Path, default=DEFAULT_SESSIONS_ROOT,
        help=f"where session dirs are written (default: {DEFAULT_SESSIONS_ROOT})",
    )
    run.add_argument("--dry-run", action="store_true", help="record the plan without booting a host")
    run.add_argument(
        "--fresh-profile", action="store_true",
        help="archive the channel's existing profile dir aside before booting",
    )
    run.add_argument("--boot-timeout-secs", type=int, default=30)
    run.add_argument("--observe-rounds", type=int, default=6)
    run.add_argument("--observe-interval-secs", type=float, default=5.0)
    run.add_argument("-v", "--verbose", action="store_true")
    return p


def main(argv: list[str] | None = None) -> int:
    args = _build_parser().parse_args(argv)
    logging.basicConfig(
        level=logging.DEBUG if args.verbose else logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s %(message)s",
    )

    fixture = Fixture.load(args.fixture)
    config = SessionConfig(
        channel=args.channel,
        fixture=fixture,
        sessions_root=args.sessions_root,
        binary=args.binary or default_binary_for(args.channel),
        dry_run=args.dry_run,
        fresh_profile=args.fresh_profile,
        boot_timeout_secs=args.boot_timeout_secs,
        observe_rounds=args.observe_rounds,
        observe_interval_secs=args.observe_interval_secs,
    )
    result = E2ESession(config).run()
    print(f"session {result.session_id} -> {result.session_dir}")
    if result.dry_run:
        print("dry-run: plan.json written, no host booted")
    else:
        print(f"ready={result.ready} outcome={result.outcome}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
