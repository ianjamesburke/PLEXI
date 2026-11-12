#!/usr/bin/env python3
"""Require a completed verify-only workflow for the exact tagged commit."""
import json
import subprocess
import sys


def gh(*args):
    return json.loads(subprocess.check_output(["gh", *args], text=True))


def verified(sha):
    runs = gh("run", "list", "--workflow", "release.yml", "--event", "workflow_dispatch", "--limit", "100", "--json", "databaseId,headSha,conclusion,event")
    for run in runs:
        if run.get("headSha") != sha or run.get("conclusion") != "success" or run.get("event") != "workflow_dispatch":
            continue
        details = gh("run", "view", str(run["databaseId"]), "--json", "jobs")
        if any(job.get("name") == f"verify-only evidence {sha}" and job.get("conclusion") == "success" for job in details.get("jobs", [])):
            print(f"release evidence: verified run {run['databaseId']} for {sha}")
            return True
    return False


if __name__ == "__main__":
    try:
        sys.exit(0 if verified(sys.argv[1]) else 1)
    except (OSError, subprocess.CalledProcessError, ValueError, KeyError, IndexError) as error:
        print(f"release evidence: could not verify GitHub workflow: {error}", file=sys.stderr)
        sys.exit(1)
