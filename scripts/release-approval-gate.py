#!/usr/bin/env python3
"""Refuse to sign unless the release environment requires a human reviewer.

Runs as the first step of every job that can read the release signing
identity. GitHub creates a referenced environment that does not exist, with no
protection rules, so the job would start without anyone approving it. This
gate reads the environment's rules and fails closed when they are missing,
when no reviewer is named, or when administrators may bypass them.
Usage: release-approval-gate.py <environment>; needs GH_TOKEN with actions:read
and GITHUB_REPOSITORY.
"""
import json
import os
import subprocess
import sys


def refuse(environment, reason):
    print(f"release-approval-gate: REFUSED environment={environment}: {reason}. "
          "Fix: repository Settings > Environments > "
          f"{environment}: add a required reviewer and untick "
          "'Allow administrators to bypass configured protection rules' "
          "(scripts/RELEASE_CHANNELS.md, Signed publish approval).", file=sys.stderr)
    return 1


def main(argv):
    if len(argv) != 2:
        print("usage: release-approval-gate.py <environment>", file=sys.stderr)
        return 2
    environment = argv[1]
    repository = os.environ.get("GITHUB_REPOSITORY")
    if not repository:
        return refuse(environment, "GITHUB_REPOSITORY is not set")
    try:
        result = subprocess.run(
            ["gh", "api", f"repos/{repository}/environments/{environment}"],
            capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired) as error:
        return refuse(environment, f"could not run gh api: {error}")
    if result.returncode != 0:
        return refuse(environment, f"gh api failed ({result.returncode}): {result.stderr.strip()}")
    try:
        body = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        return refuse(environment, f"gh api returned invalid JSON: {error}")
    if body.get("can_admins_bypass") is not False:
        return refuse(environment, "administrators may bypass its protection rules")
    reviewers = [
        reviewer
        for rule in body.get("protection_rules") or []
        if rule.get("type") == "required_reviewers"
        for reviewer in rule.get("reviewers") or []
    ]
    if not reviewers:
        return refuse(environment, "no required reviewer is configured")
    names = ", ".join(
        str((r.get("reviewer") or {}).get("login") or (r.get("reviewer") or {}).get("slug") or r.get("type"))
        for r in reviewers)
    print(f"release-approval-gate: PASS environment={environment} reviewers={names}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
