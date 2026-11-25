"""Qualify ordered batches and land each PR through the existing squash flow."""
from dataclasses import dataclass
from datetime import datetime, timezone
from fnmatch import fnmatchcase
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
from uuid import uuid4

SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS))
from alpha_lock import lock_path, writer_lock

OID = re.compile(r"[0-9a-f]{40}")
# .claude/agents/plexi-pm.md, "Gates and limits". Apply source-path guard
# filters to edits too, since path guards can depend on any changed file there.
FILTER_RULES = (
    (("src/ui/*", "src/ui_tests/*", "src/ui_tests.rs", "src/overlays/*"), ("ui::",)),
    (("src/ui/*", "src/overlays/*"), ("ui::layout_guard_tests::", "ui_tests::tests::")),
    (("src/app/quit.rs",), ("app::", "ui_tests::tests::")),
    (("src/broker/*", "src/host/plugin*", "src/plexi_ai/*", "src/pane_ops/create.rs",
      "src/*socket*", "src/*caller*", "src/host/ipc*", "src/cli/ipc*"),
     ("testing::permission_gate_tests::", "testing::socket_policy_tests")),
)
VISIBLE_PATHS = ("website/*", "plugins/*", "src/ui/*", "src/overlays/*",
                 "src/app/ui*", "src/app/quit.rs", "src/render/*", "src/workspace/render*", "assets/*")
INFRA_MARKERS = ("no space left on device", "disk pressure", "floor check failed",
                 "insufficient free space", "lock timeout", "could not execute process",
                 "failed to download", "could not resolve host")


class Refusal(RuntimeError):
    pass


class RebaseRefusal(Refusal):
    pass


@dataclass(frozen=True)
class PR:
    number: int
    branch: str
    head: str
    filters: tuple
    pty: tuple
    scenes: tuple


@dataclass(frozen=True)
class Entry:
    pr: PR
    parent: str
    tip: str
    tree: str


def matches(path, patterns):
    return any(fnmatchcase(path, pattern) for pattern in patterns)


def ordered_unique(values):
    return tuple(dict.fromkeys(values))


def required_filters(paths):
    return ordered_unique(f for patterns, filters in FILTER_RULES
                          if any(matches(path, patterns) for path in paths) for f in filters)


def validate_filters(paths, filters):
    missing = set(required_filters(paths)) - set(filters)
    if missing:
        raise Refusal("missing required filters " + ", ".join(sorted(missing)))


def block(body, title, optional=False):
    lines = body.splitlines()
    positions = [i for i, line in enumerate(lines) if line.strip() == title + ":"]
    if not positions and optional:
        return ()
    if len(positions) != 1:
        raise Refusal(f"expected one {title}: block")
    values = []
    for line in lines[positions[0] + 1:]:
        line = line.strip()
        if not line.startswith("- "):
            break
        values.append(line[2:].strip())
    if optional and values == ["none"]:
        return ()
    if not values or any(value == "none" or value.startswith("-") for value in values):
        raise Refusal(f"invalid or empty {title}: block")
    return ordered_unique(values)


def batches(items):
    for index in range(0, len(items), 4):
        yield items[index:index + 4]


def qualify(group, attempt, park, report):
    result = attempt(group)
    if result == "infra":
        report("candidate infrastructure failure; retrying the same batch once")
        result = attempt(group)
    if result == "infra":
        raise Refusal("candidate infrastructure failed twice; see candidate logs")
    if result == "fail":
        if len(group) == 1:
            park(group[0])
        else:
            midpoint = len(group) // 2
            report(f"splitting failed candidate into {midpoint} and {len(group) - midpoint}")
            qualify(group[:midpoint], attempt, park, report)
            qualify(group[midpoint:], attempt, park, report)


class Runner:
    def __init__(self, cwd, log):
        self.cwd = cwd
        self.log = log

    def run(self, *args, cwd=None, env=None, fd=None):
        result = subprocess.run(args, cwd=cwd or self.cwd, env=env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                stdin=subprocess.DEVNULL, pass_fds=() if fd is None else (fd,))
        with self.log.open("a") as output:
            print("+", *args, file=output)
            print(result.stdout, end="", file=output)
        return result

    def checked(self, *args, **kwargs):
        result = self.run(*args, **kwargs)
        if result.returncode:
            detail = result.stdout.strip().splitlines()
            raise Refusal(f"{' '.join(args[:3])} exited {result.returncode}: "
                          f"{detail[-1] if detail else 'see queue log'}")
        return result.stdout.rstrip("\n")


class Queue:
    def __init__(self, root, logs, runner):
        self.root, self.logs, self.runner = root, logs, runner
        self.worktree = root / "worktrees" / ("merge-queue-" + logs.name)
        self.merged, self.skipped, self.parked = [], {}, []
        self.active = []
        self.failure = None
        self.attempt_number = 0

    def report(self, message):
        line = "INFO merge-queue " + " ".join(message.splitlines())
        with (self.logs / "queue.log").open("a") as output:
            print(line, file=output)
        print(line, flush=True)

    def skip(self, pr, reason):
        self.skipped[pr] = reason
        self.report(f"#{pr} skipped: {reason}")

    def git(self, *args, **kwargs):
        return self.runner.checked("git", *args, **kwargs)

    def view(self, number, fields):
        return json.loads(self.runner.checked("gh", "pr", "view", str(number), "--json", fields))

    def alpha(self):
        self.git("fetch", "--quiet", "origin", "+refs/heads/alpha:refs/remotes/origin/alpha")
        return self.git("rev-parse", "refs/remotes/origin/alpha")

    def lane(self, pr):
        lane = self.root / "worktrees" / pr.branch
        if not lane.is_dir() or lane.resolve() != lane:
            raise Refusal("missing or noncanonical lane worktree")
        if (self.git("rev-parse", "--show-toplevel", cwd=lane) != str(lane)
                or self.git("symbolic-ref", "HEAD", cwd=lane) != f"refs/heads/{pr.branch}"
                or self.git("rev-parse", "HEAD", cwd=lane) != pr.head
                or self.git("status", "--porcelain", cwd=lane)):
            raise Refusal("lane must be clean, on its PR branch, and at the pinned head")
        return lane

    def check_head(self, pr, head):
        fields = "headRefOid,state,isDraft,baseRefName,reviewDecision,autoMergeRequest"
        number = pr.number
        result = self.runner.run("gh", "pr", "checks", str(number), "--json", "name,bucket")
        # gh exits 1 for failing checks and 8 for pending. No checks is
        # normal in this repository, but network and permissions errors aren't.
        if result.returncode == 1 and result.stdout.strip() == f"no checks reported on the '{pr.branch}' branch":
            rows = []
        elif result.returncode in (0, 1, 8):
            rows = json.loads(result.stdout)
        else:
            raise Refusal("could not establish GitHub check state")
        if not isinstance(rows, list) or any(row["bucket"] not in {"pass", "pending", "skipping", "neutral"} for row in rows):
            raise Refusal("GitHub reported a failing or unknown check")
        current = self.view(number, fields)
        if current.get("autoMergeRequest"):
            raise Refusal("PR has a GitHub auto-merge request")
        if (current["state"] != "OPEN" or current["isDraft"] or current["headRefOid"] != head
                or current["baseRefName"] != "alpha" or current["reviewDecision"] == "CHANGES_REQUESTED"):
            raise Refusal("PR is no longer open and ready at the pinned head")

    def admit(self, number, base):
        info = self.view(number, "headRefName,headRefOid,baseRefName,state,isDraft,isCrossRepository,body,labels,reviews,reviewDecision")
        branch, head = info["headRefName"], info["headRefOid"]
        if info["state"] != "OPEN" or info["isDraft"] or info["baseRefName"] != "alpha":
            raise Refusal("PR must be open, ready, and target alpha")
        if info["isCrossRepository"] or not OID.fullmatch(head) or not branch.startswith("feature/"):
            raise Refusal("expected a same-repository feature branch with a full pinned SHA")
        self.git("check-ref-format", f"refs/heads/{branch}")
        labels = {label["name"].lower() for label in info["labels"]}
        body = info["body"] or ""
        stint_ids = set(re.findall(r"(?:^|[-/])(?:stint-)?([0-9]{4,})(?=-|$)", branch))
        # Match the same explicit directives used by merge-pr's stint closeout.
        for line in body.splitlines():
            if re.match(r"^[\s>*_-]*stints?(?:-partial)?[\s*_]*:", line, re.IGNORECASE):
                stint_ids.update(re.findall(r"\b[0-9]+\b", line))
            stint_ids.update(re.findall(
                r"(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?) +stint[ #\-]*(\d{4})\b",
                line, re.IGNORECASE))
        task_dir = self.root / ".stint" / "tasks"
        for stint in stint_ids:
            tasks = list(task_dir.glob(f"{stint}-*.md"))
            if len(tasks) != 1:
                raise Refusal(f"cannot determine risk for stint {stint}")
            metadata = tasks[0].read_text().split("## ", 1)[0]
            if re.search(r"\brisk-high\b", metadata):
                labels.add("risk-high")
        if "risk-high" in labels:
            raise Refusal("risk-high requires independent exact-head evidence outside the queue")
        reviews = {}
        if len(info["reviews"]) >= 100:
            raise Refusal("too many GitHub reviews to establish current approval")
        for review in info["reviews"]:
            if review["state"] != "COMMENTED":
                reviews[review["author"]["login"]] = review
        if (info["reviewDecision"] == "CHANGES_REQUESTED"
                or not any(r["state"] == "APPROVED" and (r.get("commit") or {}).get("oid") == head
                           for r in reviews.values())):
            raise Refusal("no current GitHub approval at the pinned head")
        pr = PR(number, branch, head, block(body, "Candidate filters"),
                block(body, "Candidate PTY filters", optional=True),
                block(body, "Candidate scenes", optional=True))
        self.lane(pr)
        paths = self.git("diff", "--name-only", "-z", "--no-renames", f"{base}...{head}").split("\0")
        validate_filters(paths, pr.filters)
        if labels & {"ui", "visible-ui", "requires-ian", "website"} or any(matches(path, VISIBLE_PATHS) for path in paths):
            raise Refusal("potentially visible UI requires Ian's review outside the queue")
        self.check_head(pr, head)
        self.report(f"#{number} admitted at {head}; filters={pr.filters}, PTY={pr.pty}, scenes={pr.scenes}")
        return pr

    def rebase(self, head, old_base, onto):
        self.git("-c", "core.hooksPath=/dev/null", "checkout", "--quiet", "--detach", head, cwd=self.worktree)
        if self.git("rev-list", "--merges", f"{old_base}..{head}"):
            raise RebaseRefusal("PR has merge commits in its patch series")
        result = self.runner.run("git", "-c", "rerere.enabled=false", "-c", "core.hooksPath=/dev/null",
                                 "rebase", "--no-autostash", "--no-update-refs", "--no-gpg-sign",
                                 "--onto", onto, old_base, cwd=self.worktree)
        if result.returncode:
            self.git("rebase", "--abort", cwd=self.worktree)
            raise RebaseRefusal("rebase did not apply cleanly")
        tip = self.git("rev-parse", "HEAD", cwd=self.worktree)
        diff = self.git("range-diff", "--no-color", f"{old_base}..{head}", f"{onto}..{tip}",
                        cwd=self.worktree)
        if not diff or any(not re.match(r"^\s*\d+:\s+[0-9a-f]+\s+=\s+\d+:\s+[0-9a-f]+(?:\s|$)", line)
                           for line in diff.splitlines()):
            raise RebaseRefusal("range-diff is not identical")
        return tip, self.git("rev-parse", f"{tip}^{{tree}}", cwd=self.worktree)

    def stack(self, group, base):
        tip = base
        entries = []
        for pr in group:
            try:
                self.lane(pr)
                fork = self.git("merge-base", "--all", base, pr.head)
                if not OID.fullmatch(fork):
                    raise Refusal("PR and alpha must have one merge base")
                new_tip, tree = self.rebase(pr.head, fork, tip)
            except Refusal as error:
                self.skip(pr.number, str(error))
                continue
            entries.append(Entry(pr, tip, new_tip, tree))
            self.report(f"#{pr.number} stacked pinned={pr.head} onto={tip} tip={new_tip} tree={tree}")
            tip = new_tip
        if entries:
            self.git("-c", "core.hooksPath=/dev/null", "checkout", "--quiet", "--detach", tip, cwd=self.worktree)
        return entries

    def candidate(self, base, entries):
        self.attempt_number += 1
        first, last = entries[0], entries[-1]
        name = f"candidate-{self.attempt_number}"
        env = dict(os.environ)
        env.pop("PLEXI_RUN_LOGGED_ACTIVE", None)
        for var, field in (("PLEXI_PR_TEST_FILTERS", "filters"),
                           ("PLEXI_PR_REQUIRED_PTY_FILTERS", "pty"),
                           ("PLEXI_PR_REQUIRED_SCENES", "scenes")):
            env[var] = "\n".join(ordered_unique(value for entry in entries for value in getattr(entry.pr, field))) or "none"
        self.report(f"{name} base={base} heads={[e.pr.head for e in entries]} tip={last.tip}")
        result = self.runner.run(
            sys.executable, str(SCRIPTS / "run_logged.py"), "--tee", "--dir", str(self.logs),
            name, "--", "just", "_merge-queue-candidate", str(first.pr.number),
            str(self.worktree), base, last.tip, env=env,
        )
        lines = result.stdout.splitlines()
        verdict = next((line for line in reversed(lines) if line.startswith(f"RESULT name={name} ")), "")
        self.failure = (base, entries, verdict or f"missing RESULT; see {self.logs / 'commands.log'}")
        self.report(f"{name} {self.failure[2]}")
        if verdict.startswith(f"RESULT name={name} status=pass exit=0 ") and result.returncode == 0:
            return "pass"
        if (not verdict.startswith(f"RESULT name={name} status=fail exit=")
                or result.returncode >= 126 or "merge-queue: candidate gates started" not in lines
                or any(any(marker in line.lower() for marker in INFRA_MARKERS)
                       or line.startswith(("Traceback (most recent call last):", "build-cache: all persistent"))
                       for line in lines)):
            return "infra"
        return "fail"

    def park(self, pr):
        base, entries, verdict = self.failure
        with (self.logs / "parked.md").open("a") as output:
            print(f"- #{pr.number} base={base} heads={[e.pr.head for e in entries]} {verdict}", file=output)
        self.parked.append(pr.number)
        self.report(f"#{pr.number} parked after singleton candidate failure")

    def push(self, pr, head):
        lane = self.lane(pr)
        self.check_head(pr, pr.head)
        if head == pr.head:
            return head
        ref = f"refs/heads/{pr.branch}"
        # Keep the lane at the pushed head for merge-pr's cleanup. A failed
        # push can be ambiguous, so preserve the attempted head for inspection.
        self.git("reset", "--keep", head, cwd=lane)
        self.git("push", f"--force-with-lease={ref}:{pr.head}", "origin", f"{head}:{ref}", cwd=lane)
        for _ in range(30):
            current = self.view(pr.number, "headRefOid,state,isDraft")
            if current["state"] != "OPEN" or current["isDraft"] or current["headRefOid"] not in (pr.head, head):
                raise Refusal("PR changed while waiting for GitHub head registration")
            if current["headRefOid"] == head:
                self.check_head(pr, head)
                return head
            time.sleep(2)
        raise Refusal("GitHub did not register the leased push")

    def land(self, base, entries, fd=None):
        expected = base
        for index, entry in enumerate(entries):
            pr = entry.pr
            if self.alpha() != expected:
                raise Refusal(f"pre-merge base mismatch before #{pr.number}; expected {expected}")
            head, tree = (entry.tip, entry.tree) if entry.parent == expected else self.rebase(
                entry.tip, entry.parent, expected)
            if tree != entry.tree:
                raise Refusal(f"#{pr.number} landing rebase tree differs from tested tree")
            self.report(f"#{pr.number} landing head={head} base={expected} tree={tree}")
            head = self.push(pr, head)
            if self.alpha() != expected:
                raise Refusal(f"pre-merge base mismatch before #{pr.number}; expected {expected}")
            env = {**os.environ, "PLEXI_QUEUE_EXPECTED_ALPHA": expected,
                   "PLEXI_QUEUE_EXPECTED_TREE": entry.tree}
            result = self.runner.run("just", "bs-merge", str(pr.number), "--reviewed-head", head,
                                     fd=fd, env=env)
            remote = self.alpha()
            current = self.view(pr.number, "state,mergeCommit")
            if result.returncode and current["state"] == "OPEN" and remote == expected:
                self.skip(pr.number, "bs-merge refused; see commands.log")
                rest = [item.pr for item in entries[index + 1:]]
                self.report(f"fresh candidate required for {[item.number for item in rest]}")
                return rest
            merge = (current.get("mergeCommit") or {}).get("oid", "")
            if (current["state"] != "MERGED" or not OID.fullmatch(merge)
                    or self.runner.run("git", "merge-base", "--is-ancestor", merge, remote).returncode):
                raise Refusal(f"#{pr.number} merge unverified: GitHub state or alpha ancestry mismatch")
            if self.git("rev-parse", f"{merge}^{{tree}}") != entry.tree or self.git("rev-parse", f"{remote}^{{tree}}") != entry.tree:
                raise Refusal(f"#{pr.number} post-merge tree mismatch; alpha={remote}, tested={entry.tree}")
            if self.git("show", "-s", "--format=%P", merge) != expected or remote != merge:
                raise Refusal(f"#{pr.number} alpha moved outside the expected squash; alpha={remote}")
            self.merged.append(pr.number)
            self.report(f"#{pr.number} verified merged at {merge}")
            if result.returncode:
                raise Refusal(f"#{pr.number} squash verified but bs-merge closeout failed; see commands.log")
            expected = remote
        return []

    def attempt(self, group):
        try:
            base = self.alpha()
        except Refusal as error:
            self.report(str(error))
            return "infra"
        entries = self.stack([pr for pr in group if pr.number not in self.skipped], base)
        if not entries:
            return "pass"
        try:
            result = self.candidate(base, entries)
        except OSError as error:
            self.report(f"candidate infrastructure failure: {error}")
            return "infra"
        if result == "fail" and len(entries) == 1:
            self.park(entries[0].pr)
            return "pass"
        if result != "pass":
            return result
        for attempt in range(2):
            try:
                with writer_lock(lock_path()) as fd:
                    rest = self.land(base, entries, fd)
                break
            except TimeoutError as error:
                self.report(str(error))
                if attempt:
                    raise Refusal("alpha writer lock timed out twice") from error
        if rest:
            qualify(rest, self.attempt, self.park, self.report)
        return "pass"

    def execute(self, numbers):
        base = self.alpha()
        for number in dict.fromkeys(numbers):
            try:
                self.active.append(self.admit(number, base))
            except (Refusal, OSError, ValueError, KeyError, TypeError) as error:
                self.skip(number, str(error))
        if not self.active:
            return
        self.git("-c", "core.hooksPath=/dev/null", "worktree", "add", "--quiet", "--detach",
                 str(self.worktree), base)
        try:
            for group in batches(self.active):
                qualify(group, self.attempt, self.park, self.report)
        finally:
            cleanup = self.runner.run("git", "worktree", "remove", str(self.worktree))
            if cleanup.returncode:
                self.report(f"candidate worktree retained at {self.worktree}; see commands.log")

    def summary(self, numbers):
        pending = [n for n in dict.fromkeys(numbers)
                   if n not in self.merged and n not in self.skipped and n not in self.parked]
        self.report(f"merged={self.merged} skipped={self.skipped} parked={self.parked} not-landed={pending}")


def main(args):
    if not args or any(not re.fullmatch(r"[1-9][0-9]*", arg) for arg in args):
        print("usage: just merge-queue <PR>...", file=sys.stderr)
        return 2
    root = lock_path().parent.parent
    logs = root / ".stint" / "runs" / "merge-queue" / (
        datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid4().hex[:8]
    )
    logs.mkdir(parents=True)
    train = Queue(root, logs, Runner(SCRIPTS.parent, logs / "commands.log"))
    numbers = list(map(int, args))
    train.report(f"log directory {logs}")
    try:
        train.execute(numbers)
    except (Refusal, OSError, ValueError, KeyError, TypeError) as error:
        train.report(f"HALT: {error}")
        return 1
    finally:
        train.summary(numbers)
    return int(bool(train.skipped or train.parked))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
