#!/usr/bin/env python3
"""Fail-closed routing and exact-head gates for babysitter aggregate runs."""

import argparse
from datetime import datetime
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path


CONFIG = Path(__file__).resolve().parents[1] / ".agents/skills/babysitter-v3/RUN_CONFIG.toml"
REQUIRED_LOCAL = {"build", "test", "clippy"}


class PolicyError(ValueError):
    pass


def config(path=CONFIG):
    with open(path, "rb") as stream:
        return tomllib.load(stream)


def route(settings, size, exhausted=(), worker_alias=None):
    role = {"S": "worker_small", "M": "worker_medium", "L": "worker_large"}.get(size.upper())
    if role is None:
        raise PolicyError(f"unknown size: {size}")
    vendors = settings["engines"]["alias_vendor"]
    models = settings["engines"]["alias_model"]
    unavailable = set(exhausted)

    def eligible(alias):
        if alias not in vendors or alias not in models:
            raise PolicyError(f"unconfigured alias: {alias}")
        return alias not in unavailable

    if worker_alias is None:
        worker_alias = next((alias for alias in settings["roles"][role] if eligible(alias)), None)
    elif worker_alias not in settings["roles"][role] or not eligible(worker_alias):
        raise PolicyError("worker alias is unavailable for this size")
    if worker_alias is None:
        raise PolicyError(f"{role} exhausted")
    tester_alias = next((alias for alias in settings["roles"]["tester"]
                         if eligible(alias) and vendors[alias] != vendors[worker_alias]), None)
    if tester_alias is None:
        raise PolicyError("independent tester pool exhausted")
    return {"size": size.upper(), "worker": worker_alias, "worker_model": models[worker_alias],
            "worker_vendor": vendors[worker_alias], "tester": tester_alias,
            "tester_model": models[tester_alias], "tester_vendor": vendors[tester_alias]}


def check_gate(settings, record, pr, checks, base_head, phase):
    if settings["topology"]["mode"] != "aggregate" or not settings["authorization"]["auto_merge"]:
        raise PolicyError("aggregate merge-all policy is not configured")
    expected_base = (settings["topology"]["branch"] if phase == "batch"
                     else settings["topology"]["final_base"])
    if phase not in ("batch", "final") or pr["baseRefName"] != expected_base:
        raise PolicyError("wrong PR base for phase")
    head = pr["headRefOid"]
    if phase == "final" and (record.get("aggregate_head") != head
                             or not record.get("integrated_batches")):
        raise PolicyError("final record does not identify integrated aggregate head")
    if not head or record.get("head") != head or record.get("base_head") != base_head:
        raise PolicyError("PR head or base changed since validation")
    if record.get("size") not in ("S", "M", "L"):
        raise PolicyError("missing batch size")
    assigned = route(settings, record["size"], record.get("exhausted", []),
                     record.get("worker_alias"))
    if (record.get("worker_alias") != assigned["worker"]
            or record.get("tester_alias") != assigned["tester"]):
        raise PolicyError("recorded route differs from policy")
    if record.get("local", {}).get("head") != head or not REQUIRED_LOCAL.issubset(
        {key for key, value in record.get("local", {}).items() if value is True}
    ):
        raise PolicyError("local build, test, and clippy must pass at PR head")
    tested = record.get("validation", {})
    if tested.get("head") != head or tested.get("verdict") != "PASS":
        raise PolicyError("independent validator did not pass PR head")
    if (tested.get("worker_vendor") != assigned["worker_vendor"]
            or tested.get("tester_vendor") != assigned["tester_vendor"]):
        raise PolicyError("self-validation is forbidden")
    if tested.get("install_head") != head:
        raise PolicyError("installed PR build does not match head")
    if record.get("checks_head") != head:
        raise PolicyError("CI record does not match head")
    waivers = record.get("waivers", [])
    for waiver in waivers:
        if (waiver.get("kind") != "pre_existing_alpha" or not waiver.get("evidence")
                or not waiver.get("ruling") or waiver.get("approved_by") != "Ian"
                or not waiver.get("check_name") or not waiver.get("alpha_head")
                or not waiver.get("ruling_at") or not record.get("dispatched_at")):
            raise PolicyError("waiver lacks a pre-dispatch human ruling")
        try:
            if datetime.fromisoformat(waiver["ruling_at"]) >= datetime.fromisoformat(record["dispatched_at"]):
                raise PolicyError("waiver ruling came after dispatch")
        except ValueError as error:
            raise PolicyError("waiver has invalid timestamps") from error
    if not checks:
        raise PolicyError("PR checks are missing")
    for check in checks:
        if check.get("bucket") == "pass":
            continue
        if check.get("bucket") != "fail" or not any(
            waiver["check_name"] == check.get("name") for waiver in waivers
        ):
            raise PolicyError("PR check failed or is pending without an explicit waiver")
    return head


def command(*args):
    result = subprocess.run(args, check=True, capture_output=True, text=True)
    return result.stdout.strip()


def alpha_root():
    current = None
    for line in command("git", "worktree", "list", "--porcelain").splitlines():
        if line.startswith("worktree "):
            current = line[len("worktree "):]
        elif line == "branch refs/heads/alpha":
            return Path(current)
    raise PolicyError("alpha worktree not found")


def read_checks(pr_number):
    # `gh pr checks` exits nonzero for red checks, even though its JSON is valid.
    result = subprocess.run(("gh", "pr", "checks", str(pr_number), "--json", "name,bucket"),
                            check=False, capture_output=True, text=True)
    if not result.stdout.strip():
        raise PolicyError(f"cannot read PR checks: {result.stderr.strip()}")
    return json.loads(result.stdout)


def preflight(settings, task_root, sprint_file, requested):
    if settings["topology"] != {"mode": "aggregate", "branch": "feature/alpha-cleanup",
                                 "final_base": "alpha", "require_final_install": True}:
        raise PolicyError("aggregate topology is unresolved or incomplete")
    members = re.findall(r"\[(\d{4})\]\(\.\./tasks/", sprint_file.read_text())
    if not members or len(members) != len(set(members)) or "0991" not in members:
        raise PolicyError("invalid Alpha cleanup sprint membership")
    task_data = {}
    for task_id in members:
        files = list(task_root.glob(f"{task_id}-*.md"))
        if len(files) != 1:
            raise PolicyError(f"missing or duplicate task {task_id}")
        body = files[0].read_text()
        metadata = body.split("---", 2)
        if len(metadata) < 3:
            raise PolicyError(f"invalid task metadata {task_id}")
        status = re.search(r"(?m)^status: (\w+)$", metadata[1])
        size = re.search(r"(?m)^size: ([sml])$", metadata[1])
        blockers = re.search(r"(?ms)^blocked_by:\s*\n((?:  - \d+\n)*)", metadata[1])
        task_data[task_id] = {"status": status.group(1) if status else None,
                              "size": size.group(1).upper() if size else None,
                              "blocked_by": [f"{int(item):04d}" for item in
                                             re.findall(r"(?m)^  - (\d+)$", blockers.group(1) if blockers else "")],
                              "done_when": "## Done-When" in body}
    if task_data["0991"]["blocked_by"]:
        raise PolicyError("preflight task has a blocking dependency")
    for task_id, task in task_data.items():
        if not task["done_when"] or not task["size"] or (task_id != "0991" and "0991" not in task["blocked_by"]):
            raise PolicyError(f"task {task_id} is missing Done-When or 0991 dependency")
    visiting = set()
    visited = set()

    def visit(task_id):
        if task_id in visiting:
            raise PolicyError("Alpha cleanup task graph contains a cycle")
        if task_id in visited:
            return
        visiting.add(task_id)
        for blocker in task_data[task_id]["blocked_by"]:
            if blocker in task_data:
                visit(blocker)
        visiting.remove(task_id)
        visited.add(task_id)

    for task_id in task_data:
        visit(task_id)
    if task_data["0991"]["status"] != "done":
        raise PolicyError("0991 has not completed; product dispatch is blocked")
    selected = [f"{int(task_id):04d}" for task_id in requested]
    if not selected or any(task_id == "0991" or task_id not in task_data for task_id in selected):
        raise PolicyError("requested task is missing or not a product task")
    if any(task_data[task_id]["status"] not in ("todo", "in_progress") for task_id in selected):
        raise PolicyError("requested task is not claimable")
    for task_id in selected:
        for blocker in task_data[task_id]["blocked_by"]:
            if blocker in task_data and task_data[blocker]["status"] != "done":
                raise PolicyError(f"task {task_id} is blocked by {blocker}")
    return {"topology": settings["topology"], "tasks": selected,
            "size": max((task_data[task_id]["size"] for task_id in selected),
                        key=lambda item: {"S": 1, "M": 2, "L": 3}[item]),
            "preflight": "0991", "preflight_status": "done"}


def live_gate(settings, record, pr_number, phase):
    before = json.loads(command("gh", "pr", "view", str(pr_number), "--json",
                                "baseRefName,headRefOid,state"))
    if before["state"] != "OPEN":
        raise PolicyError("PR is not open")
    base_head = command("git", "ls-remote", "origin", f"refs/heads/{before['baseRefName']}").split()[0]
    checks = read_checks(pr_number)
    head = check_gate(settings, record, before, checks, base_head, phase)
    if phase == "final":
        branch = settings["topology"]["branch"]
        aggregate_remote = command("git", "ls-remote", "origin", f"refs/heads/{branch}").split()[0]
        if aggregate_remote != head:
            raise PolicyError("aggregate branch differs from final PR head")
    after = json.loads(command("gh", "pr", "view", str(pr_number), "--json",
                               "baseRefName,headRefOid,state"))
    if after != before:
        raise PolicyError("PR changed during final gate")
    final_base = command("git", "ls-remote", "origin", f"refs/heads/{before['baseRefName']}").split()[0]
    if final_base != base_head:
        raise PolicyError("base changed during final gate")
    return head


def integrate(settings, record, pr_number, manifest_path):
    """Squash one validated batch into the aggregate branch, never alpha."""
    if not record.get("stints") or not all(re.fullmatch(r"\d{4}", task_id) for task_id in record["stints"]):
        raise PolicyError("batch record has no valid stint IDs")
    root = alpha_root()
    plan = preflight(settings, root / ".stint/tasks", root / ".stint/sprints/s03.md", record["stints"])
    if record.get("size") != plan["size"]:
        raise PolicyError("record size does not match the largest stint")
    if manifest_path.exists():
        manifest = json.loads(manifest_path.read_text())
    else:
        manifest = {"topology": "aggregate", "batches": []}
    if manifest.get("topology") != "aggregate" or any(
        item.get("pr") == pr_number for item in manifest.get("batches", [])
    ):
        raise PolicyError("invalid or duplicate aggregate manifest entry")
    branch = settings["topology"]["branch"]
    locations = command("git", "worktree", "list", "--porcelain").splitlines()
    worktree = None
    current = None
    for line in locations:
        if line.startswith("worktree "):
            current = line[len("worktree "):]
        elif line == f"branch refs/heads/{branch}":
            worktree = current
            break
    if not worktree or command("git", "-C", worktree, "status", "--porcelain"):
        raise PolicyError("aggregate worktree is missing or dirty")
    remote_before = command("git", "ls-remote", "origin", f"refs/heads/{branch}").split()[0]
    if command("git", "-C", worktree, "rev-parse", "HEAD") != remote_before:
        raise PolicyError("aggregate worktree differs from remote base")
    head = live_gate(settings, record, pr_number, "batch")
    # GitHub rejects a changed head atomically. No rebase happens after validation.
    command("gh", "pr", "merge", str(pr_number), "--squash", "--match-head-commit", head)
    merged = json.loads(command("gh", "pr", "view", str(pr_number), "--json", "state,mergeCommit"))
    if merged.get("state") != "MERGED":
        raise PolicyError("PR merge did not complete")
    command("git", "-C", worktree, "fetch", "origin", branch)
    command("git", "-C", worktree, "merge", "--ff-only", f"origin/{branch}")
    aggregate_head = command("git", "-C", worktree, "rev-parse", "HEAD")
    parent_head = command("git", "-C", worktree, "rev-parse", "HEAD^")
    if (aggregate_head == remote_before or aggregate_head != merged.get("mergeCommit", {}).get("oid")
            or parent_head != remote_before):
        raise PolicyError("aggregate head does not match the merged commit")
    manifest["batches"].append({"pr": pr_number, "source_head": head,
                                 "base_before": remote_before, "aggregate_head": aggregate_head,
                                 "stints": record.get("stints", [])})
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    temporary = manifest_path.with_suffix(".tmp")
    temporary.write_text(json.dumps(manifest, indent=2) + "\n")
    temporary.replace(manifest_path)
    return aggregate_head


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    route_parser = sub.add_parser("route")
    route_parser.add_argument("size", choices=("S", "M", "L"))
    route_parser.add_argument("--exhausted", nargs="*", default=[])
    route_parser.add_argument("--worker-alias")
    gate_parser = sub.add_parser("gate")
    gate_parser.add_argument("phase", choices=("batch", "final"))
    gate_parser.add_argument("pr", type=int)
    gate_parser.add_argument("record", type=Path)
    preflight_parser = sub.add_parser("preflight")
    preflight_parser.add_argument("tasks", nargs="+")
    merge_parser = sub.add_parser("integrate")
    merge_parser.add_argument("pr", type=int)
    merge_parser.add_argument("record", type=Path)
    merge_parser.add_argument("manifest", type=Path)
    args = parser.parse_args()
    settings = config()
    try:
        if args.action == "route":
            print(json.dumps(route(settings, args.size, args.exhausted, args.worker_alias)))
        elif args.action == "preflight":
            root = alpha_root()
            print(json.dumps(preflight(settings, root / ".stint/tasks",
                                       root / ".stint/sprints/s03.md", args.tasks)))
        elif args.action == "gate":
            record = json.loads(args.record.read_text())
            print(live_gate(settings, record, args.pr, args.phase))
        else:
            record = json.loads(args.record.read_text())
            print(integrate(settings, record, args.pr, args.manifest))
    except (PolicyError, KeyError, IndexError, subprocess.CalledProcessError) as error:
        print(f"babysitter-policy: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
