#!/usr/bin/env python3
"""Fail-closed routing and exact-head gates for babysitter aggregate runs."""

import argparse
import fcntl
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

    if worker_alias is not None and (worker_alias not in settings["roles"][role]
                                     or not eligible(worker_alias)):
        raise PolicyError("worker alias is unavailable for this size")
    candidates = [worker_alias] if worker_alias else settings["roles"][role]
    selected = next(((worker, tester) for worker in candidates if eligible(worker)
                     for tester in settings["roles"]["tester"]
                     if eligible(tester) and vendors[tester] != vendors[worker]), None)
    if selected is None:
        raise PolicyError("no available independent worker/tester pair")
    worker_alias, tester_alias = selected
    return {"size": size.upper(), "worker": worker_alias, "worker_model": models[worker_alias],
            "worker_vendor": vendors[worker_alias], "tester": tester_alias,
            "tester_model": models[tester_alias], "tester_vendor": vendors[tester_alias]}


def ledger_summary(run_dir):
    """Per worker alias stats from <run_dir>/LEDGER.jsonl (one line per tester verdict or respawn)."""
    path = Path(run_dir) / "LEDGER.jsonl"
    try:
        lines = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
    except (OSError, ValueError) as error:
        raise PolicyError(f"cannot read ledger {path}: {error}") from error
    by_alias = {}
    for entry in lines:
        by_alias.setdefault(entry["worker_alias"], []).append(entry)
    summary = {}
    for alias, entries in by_alias.items():
        batches = {entry["batch"] for entry in entries}
        first = {entry["batch"] for entry in entries if entry["round"] == 1 and entry["verdict"] == "PASS"}
        green = [entry["round"] for entry in entries if entry["verdict"] == "PASS"]
        summary[alias] = {
            "attempts": len(entries), "batches": len(batches),
            "first_round_pass": {"count": len(first), "rate": round(len(first) / len(batches), 4)},
            "avg_rounds_to_green": round(sum(green) / len(green), 2) if green else None,
            "parks": sum(entry["outcome"] == "parked" for entry in entries),
            "crossovers": sum(entry["outcome"] == "crossed_over" for entry in entries),
            "usage_limit_hits": sum(bool(entry["usage_limit_hit"]) for entry in entries),
            "wall_minutes": round(sum(entry["wall_minutes"] for entry in entries), 1),
            "follow_up_stints": sum(len(entry.get("follow_up_stints") or []) for entry in entries)}
    return summary


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
    if record.get("waivers"):
        raise PolicyError("automated merge cannot authenticate a human waiver; park for explicit ruling")
    host_ci = settings.get("ci", {}).get("rust_host", "pr")
    if host_ci not in ("pr", "after_alpha"):
        raise PolicyError("unknown Rust host CI policy")
    checks = [check for check in checks if check.get("app_slug") != "coderabbitai"]
    if host_ci == "after_alpha":
        if record.get("local", {}).get("test_scope") != "full":
            raise PolicyError("post-Alpha host CI requires full local suite evidence")
        # Only the identified workflow is deferred, never a similarly named job.
        checks = [check for check in checks if check.get("workflow") != "Rust host"]
    elif not checks:
        raise PolicyError("PR checks are missing")
    for check in checks:
        if check.get("bucket") != "pass":
            raise PolicyError("PR check failed or is pending; park for an explicit ruling")
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


def read_checks(pr_number, head):
    # `gh pr checks` exits nonzero for red checks, even though its JSON is valid.
    result = subprocess.run(("gh", "pr", "checks", str(pr_number), "--json", "name,bucket,workflow,link"),
                            check=False, capture_output=True, text=True)
    if not result.stdout.strip():
        raise PolicyError(f"cannot read PR checks: {result.stderr.strip()}")
    checks = json.loads(result.stdout)
    pages = json.loads(command("gh", "api", "--paginate", "--slurp",
                               f"repos/{{owner}}/{{repo}}/commits/{head}/check-runs?per_page=100"))
    runs = [run for page in pages for run in page["check_runs"]]
    # A unique name/URL match ties a gh check to the exact-head provider. Ambiguous
    # matches retain the check and therefore cannot silently waive a failure.
    for check in checks:
        key = (check.get("name"), check.get("link"))
        matching_checks = [item for item in checks if (item.get("name"), item.get("link")) == key]
        matching_runs = [run for run in runs if (run.get("name"), run.get("details_url")) == key]
        if len(matching_checks) == len(matching_runs) == 1:
            check["app_slug"] = (matching_runs[0].get("app") or {}).get("slug")
    return checks


def preflight(settings, task_root, sprint_file, requested):
    required = {"mode": "aggregate", "branch": "feature/alpha-cleanup",
                "final_base": "alpha", "require_final_install": True}
    if any(settings["topology"].get(key) != value for key, value in required.items()):
        raise PolicyError("aggregate topology is unresolved or incomplete")
    members = re.findall(r"\[(\d{4})\]\(\.\./tasks/", sprint_file.read_text())
    if not members or len(members) != len(set(members)):
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
        status = re.search(r"(?m)^status: ([\w-]+)$", metadata[1])
        size = re.search(r"(?m)^size: ([sml])$", metadata[1])
        blockers = re.search(r"(?ms)^blocked_by:\s*\n((?:  - \d+\n)*)", metadata[1])
        task_data[task_id] = {"status": status.group(1) if status else None,
                              "size": size.group(1).upper() if size else None,
                              "blocked_by": [f"{int(item):04d}" for item in
                                             re.findall(r"(?m)^  - (\d+)$", blockers.group(1) if blockers else "")],
                              "done_when": bool(re.search(r"(?m)^## Done[- ]When\s*$", metadata[2]))}
    gate_id = settings["topology"].get("preflight_task")
    gate = task_data.get(gate_id)
    if gate and gate["blocked_by"]:
        raise PolicyError("preflight task has a blocking dependency")
    selected = [f"{int(task_id):04d}" for task_id in requested]
    if not selected or any(task_id == gate_id or task_id not in task_data for task_id in selected):
        raise PolicyError("requested task is missing or not a product task")
    for task_id in selected + ([gate_id] if gate else []):
        task = task_data[task_id]
        if not task["done_when"] or not task["size"]:
            raise PolicyError(f"task {task_id} is missing Done-When or size")
        if gate and gate["status"] != "done" and task_id != gate_id and gate_id not in task["blocked_by"]:
            raise PolicyError(f"task {task_id} is missing {gate_id} dependency")
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
    if gate and gate["status"] != "done":
        raise PolicyError(f"{gate_id} has not completed; product dispatch is blocked")
    if any(task_data[task_id]["status"] not in ("todo", "in-progress") for task_id in selected):
        raise PolicyError("requested task is not claimable")
    for task_id in selected:
        for blocker in task_data[task_id]["blocked_by"]:
            if blocker in task_data and task_data[blocker]["status"] != "done":
                raise PolicyError(f"task {task_id} is blocked by {blocker}")
    return {"topology": settings["topology"], "tasks": selected,
            "size": max((task_data[task_id]["size"] for task_id in selected),
                        key=lambda item: {"S": 1, "M": 2, "L": 3}[item]),
            "preflight": gate_id, "preflight_status": "done" if gate else "skipped"}


def verify_manifest(manifest, record, final_head, allow_unresolved=False):
    if manifest.get("topology") != "aggregate" or not manifest.get("requested_stints"):
        raise PolicyError("aggregate manifest is missing its requested queue")
    batches = manifest.get("batches", [])
    if not batches or record.get("integrated_batches") != [item.get("pr") for item in batches]:
        raise PolicyError("final record omits or reorders integrated batches")
    if len({item["pr"] for item in batches}) != len(batches):
        raise PolicyError("aggregate manifest repeats a PR")
    if any(item.get("base_race") and item.get("recovery_head") != final_head
           for item in batches) and not allow_unresolved:
        raise PolicyError("aggregate contains an integration with a changed base; park for recovery")
    for previous, current in zip(batches, batches[1:]):
        if current.get("base_before") != previous.get("aggregate_head"):
            raise PolicyError("aggregate commit chain is broken")
    if batches[-1].get("aggregate_head") != final_head:
        raise PolicyError("final installed head differs from aggregate manifest")
    integrated = [task_id for item in batches for task_id in item.get("stints", [])]
    parked = manifest.get("parked", [])
    if any(not item.get("reason") or not item.get("stint") for item in parked):
        raise PolicyError("parked stint lacks a reason")
    accounted = integrated + [item["stint"] for item in parked]
    requested = manifest["requested_stints"]
    if len(accounted) != len(set(accounted)) or sorted(accounted) != sorted(requested):
        raise PolicyError("aggregate manifest does not account for the full requested queue")
    return True


def live_gate(settings, record, pr_number, phase, manifest=None, allow_unresolved=False):
    before = json.loads(command("gh", "pr", "view", str(pr_number), "--json",
                                "baseRefName,headRefOid,state"))
    if before["state"] != "OPEN":
        raise PolicyError("PR is not open")
    base_head = command("git", "ls-remote", "origin", f"refs/heads/{before['baseRefName']}").split()[0]
    checks = read_checks(pr_number, before["headRefOid"])
    head = check_gate(settings, record, before, checks, base_head, phase)
    if phase == "final":
        if manifest is None:
            raise PolicyError("final gate requires the aggregate manifest")
        sprint_file = alpha_root() / settings["topology"]["sprint"]
        actual_queue = [task_id for task_id in re.findall(
            r"\[(\d{4})\]\(\.\./tasks/", sprint_file.read_text()
        ) if task_id != settings["topology"].get("preflight_task")]
        if sorted(manifest.get("requested_stints", [])) != sorted(actual_queue):
            raise PolicyError("aggregate manifest omits sprint tasks")
        verify_manifest(manifest, record, head, allow_unresolved)
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
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    with open(manifest_path.with_suffix(".lock"), "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        return integrate_locked(settings, record, pr_number, manifest_path)


def recover_base_race(settings, record, pr_number, manifest_path):
    """Resolve a recorded base race only after full combined installed validation."""
    with open(manifest_path.with_suffix(".lock"), "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        manifest = json.loads(manifest_path.read_text())
        raced = [item for item in manifest.get("batches", [])
                 if item.get("base_race") and not item.get("recovery_head")]
        if not raced:
            raise PolicyError("manifest has no unresolved base race")
        head = live_gate(settings, record, pr_number, "final", manifest, allow_unresolved=True)
        for item in raced:
            item["recovery_head"] = head
            item["recovery_validator"] = record["tester_alias"]
        temporary = manifest_path.with_suffix(".tmp")
        temporary.write_text(json.dumps(manifest, indent=2) + "\n")
        temporary.replace(manifest_path)
        return head


REAP_SCRIPTS = ("reap-merged-worktrees.sh", "clean-stale-targets.sh", "channel-clean-merged.sh")


def reap_after_merge(root):
    """Retire merged artifacts without failing an already-recorded integration."""
    for script in REAP_SCRIPTS:
        try:
            command("bash", str(root / "scripts" / script))
        except (subprocess.CalledProcessError, OSError) as error:
            print(f"warn: cleanup {script} failed: {error}", file=sys.stderr)


def integrate_locked(settings, record, pr_number, manifest_path):
    """Squash one validated batch into the aggregate branch, never alpha."""
    if not record.get("stints") or not all(re.fullmatch(r"\d{4}", task_id) for task_id in record["stints"]):
        raise PolicyError("batch record has no valid stint IDs")
    root = alpha_root()
    plan = preflight(settings, root / ".stint/tasks", root / settings["topology"]["sprint"], record["stints"])
    if record.get("size") != plan["size"]:
        raise PolicyError("record size does not match the largest stint")
    if manifest_path.exists():
        manifest = json.loads(manifest_path.read_text())
    else:
        members = re.findall(r"\[(\d{4})\]\(\.\./tasks/",
                             (root / settings["topology"]["sprint"]).read_text())
        manifest = {"topology": "aggregate", "requested_stints":
                    [task_id for task_id in members if task_id != settings["topology"].get("preflight_task")],
                    "batches": [], "parked": []}
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
    snapshot = json.loads(command("gh", "pr", "view", str(pr_number), "--json",
                                  "state,baseRefName,headRefOid,mergeCommit"))
    if snapshot.get("state") == "MERGED":
        # Recover a merge that succeeded before the caller could write its manifest.
        remote_before = record["base_head"]
        head = check_gate(settings, record, snapshot, read_checks(pr_number, snapshot["headRefOid"]), remote_before, "batch")
        merged = snapshot
    elif snapshot.get("state") == "OPEN":
        remote_before = command("git", "ls-remote", "origin", f"refs/heads/{branch}").split()[0]
        if command("git", "-C", worktree, "rev-parse", "HEAD") != remote_before:
            raise PolicyError("aggregate worktree differs from remote base")
        head = live_gate(settings, record, pr_number, "batch")
        if command("git", "ls-remote", "origin", f"refs/heads/{branch}").split()[0] != remote_before:
            raise PolicyError("aggregate base changed before merge")
        # GitHub rejects a changed head atomically. No rebase happens after validation.
        try:
            command("gh", "pr", "merge", str(pr_number), "--squash", "--match-head-commit", head)
        except subprocess.CalledProcessError:
            # A network error may arrive after GitHub accepted the merge.
            pass
        merged = json.loads(command("gh", "pr", "view", str(pr_number), "--json",
                                    "state,mergeCommit"))
    else:
        raise PolicyError("PR is neither open nor merged")
    if merged.get("state") != "MERGED":
        raise PolicyError("PR merge did not complete")
    command("git", "-C", worktree, "fetch", "origin", branch)
    command("git", "-C", worktree, "merge", "--ff-only", f"origin/{branch}")
    aggregate_tip = command("git", "-C", worktree, "rev-parse", "HEAD")
    aggregate_head = merged.get("mergeCommit", {}).get("oid")
    if not aggregate_head or aggregate_head == remote_before:
        raise PolicyError("GitHub did not identify the merged commit")
    parent_head = command("git", "-C", worktree, "rev-parse", f"{aggregate_head}^")
    external_advance = aggregate_tip != aggregate_head
    manifest["batches"].append({"pr": pr_number, "source_head": head,
                                 "base_before": remote_before, "aggregate_head": aggregate_head,
                                 "actual_parent": parent_head,
                                 "base_race": parent_head != remote_before or external_advance,
                                 "stints": record.get("stints", [])})
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    temporary = manifest_path.with_suffix(".tmp")
    temporary.write_text(json.dumps(manifest, indent=2) + "\n")
    temporary.replace(manifest_path)
    reap_after_merge(root)
    if parent_head != remote_before or external_advance:
        raise PolicyError("PR merged after the base moved; integration recorded for recovery")
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
    gate_parser.add_argument("--manifest", type=Path)
    ledger_parser = sub.add_parser("ledger-summary")
    ledger_parser.add_argument("run_dir", type=Path)
    preflight_parser = sub.add_parser("preflight")
    preflight_parser.add_argument("tasks", nargs="+")
    merge_parser = sub.add_parser("integrate")
    merge_parser.add_argument("pr", type=int)
    merge_parser.add_argument("record", type=Path)
    merge_parser.add_argument("manifest", type=Path)
    recovery_parser = sub.add_parser("recover-base-race")
    recovery_parser.add_argument("pr", type=int)
    recovery_parser.add_argument("record", type=Path)
    recovery_parser.add_argument("manifest", type=Path)
    args = parser.parse_args()
    settings = config()
    try:
        if args.action == "route":
            print(json.dumps(route(settings, args.size, args.exhausted, args.worker_alias)))
        elif args.action == "ledger-summary":
            print(json.dumps(ledger_summary(args.run_dir), indent=2))
        elif args.action == "preflight":
            root = alpha_root()
            print(json.dumps(preflight(settings, root / ".stint/tasks",
                                       root / settings["topology"]["sprint"], args.tasks)))
        elif args.action == "gate":
            record = json.loads(args.record.read_text())
            manifest = json.loads(args.manifest.read_text()) if args.manifest else None
            print(live_gate(settings, record, args.pr, args.phase, manifest))
        elif args.action == "integrate":
            record = json.loads(args.record.read_text())
            print(integrate(settings, record, args.pr, args.manifest))
        else:
            record = json.loads(args.record.read_text())
            print(recover_base_race(settings, record, args.pr, args.manifest))
    except (PolicyError, KeyError, IndexError, OSError, subprocess.CalledProcessError) as error:
        print(f"babysitter-policy: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
