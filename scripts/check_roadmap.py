#!/usr/bin/env python3
"""Compare the public roadmap with the stints that deliver it (stint 1437).

Reads website/src/lib/roadmap.json and the stints through the `stint` CLI
(`stint list --all --json`), never the files under .stint/.

Drift (exit 1):
  - every linked stint is done but the item is still "planned"
  - the item is "shipped" while a linked stint is still open
  - a linked stint is archived, or does not exist
  - an item or release is "shipped" in a release after the current one
    (`currentSlug`, the release a visitor can install today)

Information (exit 0): items whose linked stints are all done and named in a
commit message on the commits ref, so they ship with the release cut from it.
Mark them "shipped" in roadmap.json once that release is published.

Two inputs, read separately:
  --roadmap-ref REF  read the roadmap committed on REF. The site deploys from
                     alpha, so promotion passes origin/alpha: the public copy.
  --commits-ref REF  the release candidate whose commits are searched
                     (origin/alpha for promote beta, origin/beta for main).
Either one omitted: the working tree roadmap, or commits on HEAD.
--roadmap PATH reads that file instead of either roadmap source (tests).

Errors (exit 2): no `stint` on PATH, no .stint/ above the repo, unreadable
or wrongly shaped roadmap or `stint list` output, unknown ref.

Usage: check_roadmap.py [--roadmap-ref REF] [--commits-ref REF] [--roadmap PATH]
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import AbstractSet

REPO = Path(__file__).resolve().parent.parent
DEFAULT_ROADMAP = REPO / "website/src/lib/roadmap.json"
EDIT_HINT = "website/src/lib/roadmap.json"
ITEM_STATUSES = {"shipped", "in progress", "planned"}
RELEASE_STATUSES = {"shipped", "in progress", "next", "planned", "after 1.0"}
STINT_ID = re.compile(r"^\d{4}$")
# Commit messages are free text. The repo's convention names stints as
# "stint 1419", "stints 1127, 1259, 1260", "stints 1363 and 1358" or a
# "Stint: 0560, 0561" trailer. Only ids in such a run count, so years, PR
# numbers and digits inside shas do not.
STINT_RUN = re.compile(r"\bstints?:?\s+(\d{4}(?:(?:\s*,\s*|\s+and\s+|\s*,\s*and\s+)\d{4})*)(?![\w-])", re.IGNORECASE)
ID_TOKEN = re.compile(r"\d{4}")


class CheckError(Exception):
    """The check could not run. The message names what failed and where to fix it."""


@dataclass(frozen=True)
class Stint:
    id: str
    state: str
    title: str

    def label(self) -> str:
        return f"stint {self.id} ({self.title})"


def load_roadmap(path: Path) -> dict:
    try:
        text = path.read_text()
    except OSError as e:
        raise CheckError(f"cannot read roadmap {path}: {e}") from e
    return parse_roadmap(text, str(path))


def load_roadmap_at(ref: str, repo: Path | None = None) -> dict:
    spec = f"{ref}:{EDIT_HINT}"
    try:
        proc = subprocess.run(["git", "-C", str(repo or REPO), "show", spec], capture_output=True, text=True, check=False)
    except OSError as e:
        raise CheckError(f"`git show {spec}` failed to start: {e}") from e
    if proc.returncode != 0:
        raise CheckError(f"`git show {spec}` exited {proc.returncode}: {proc.stderr.strip()} (fetch the ref, or check the roadmap was committed there)")
    return parse_roadmap(proc.stdout, spec)


def parse_roadmap(text: str, path: str) -> dict:
    try:
        data = json.loads(text)
    except json.JSONDecodeError as e:
        raise CheckError(f"cannot parse roadmap {path}: {e}") from e
    if not isinstance(data, dict) or not isinstance(data.get("releases"), list) or not isinstance(data.get("currentSlug"), str):
        raise CheckError(f"{path}: must be an object with a 'currentSlug' string and a 'releases' list")
    for r in data["releases"]:
        if not isinstance(r, dict) or not isinstance(r.get("slug"), str) or not isinstance(r.get("items", []), list):
            raise CheckError(f"{path}: every release must be an object with a 'slug' string and an 'items' list; got {r!r:.80}")
        if r.get("status") not in RELEASE_STATUSES:
            raise CheckError(f"{path}: release '{r['slug']}' has unknown status {r.get('status')!r}")
        for item in r.get("items", []):
            if not isinstance(item, dict) or not isinstance(item.get("text"), str) or not isinstance(item.get("stints", []), list):
                raise CheckError(f"{path}: release '{r['slug']}' has an item that is not an object with a 'text' string and an optional 'stints' list; got {item!r:.80}")
            if item.get("status") not in ITEM_STATUSES:
                raise CheckError(f"{path}: item {item['text']!r} has unknown status {item.get('status')!r}")
            for sid in item.get("stints", []):
                if not isinstance(sid, str) or not STINT_ID.match(sid):
                    raise CheckError(f"{path}: item {item['text']!r} lists stint {sid!r}; ids are four-digit strings like \"0577\"")
    if data["currentSlug"] not in [r["slug"] for r in data["releases"]]:
        raise CheckError(f"{path}: currentSlug '{data['currentSlug']}' names no release")
    return data


def load_stints() -> dict[str, Stint]:
    exe = shutil.which("stint")
    if exe is None:
        raise CheckError("the `stint` CLI is not on PATH; install it (cargo install stint) and run from the release owner's checkout")
    try:
        proc = subprocess.run([exe, "list", "--all", "--json"], cwd=REPO, capture_output=True, text=True, check=False)
    except OSError as e:
        raise CheckError(f"`stint list --all --json` failed to start: {e}") from e
    if proc.returncode != 0:
        raise CheckError(f"`stint list --all --json` exited {proc.returncode} in {REPO}: {proc.stderr.strip()} (the roadmap check needs the .stint/ directory of the release owner's checkout)")
    return parse_stints(proc.stdout)


def parse_stints(text: str) -> dict[str, Stint]:
    try:
        rows = json.loads(text)
    except json.JSONDecodeError as e:
        raise CheckError(f"`stint list --all --json` printed invalid JSON: {e}") from e
    if not isinstance(rows, list):
        raise CheckError(f"`stint list --all --json` printed a {type(rows).__name__}, expected a list of tasks; check `stint --version`")
    out: dict[str, Stint] = {}
    for row in rows:
        if not isinstance(row, dict) or not all(isinstance(row.get(k), str) for k in ("id", "state", "title")):
            raise CheckError(f"`stint list --all --json` printed a task without string 'id', 'state' and 'title': {row!r:.120}; check `stint --version`")
        out[row["id"]] = Stint(row["id"], row["state"], row["title"])
    return out


def stints_named_on(ref: str, repo: Path | None = None) -> set[str]:
    try:
        proc = subprocess.run(["git", "-C", str(repo or REPO), "log", "--format=%B%x00", ref, "--"], capture_output=True, text=True, check=False)
    except OSError as e:
        raise CheckError(f"`git log {ref}` failed to start: {e}") from e
    if proc.returncode != 0:
        raise CheckError(f"`git log {ref}` exited {proc.returncode}: {proc.stderr.strip()} (fetch the ref or pass another --commits-ref)")
    return stint_ids_in(proc.stdout)


def stint_ids_in(text: str) -> set[str]:
    named: set[str] = set()
    for run in STINT_RUN.findall(text):
        named.update(ID_TOKEN.findall(run))
    return named


def check(data: dict, stints: dict[str, Stint], named: AbstractSet[str], ref: str) -> tuple[list[str], list[str]]:
    drift: list[str] = []
    info: list[str] = []
    slugs = [r["slug"] for r in data["releases"]]
    current = slugs.index(data["currentSlug"])
    for index, rel in enumerate(data["releases"]):
        where = f"release '{rel['slug']}'"
        future = index > current
        if future and rel["status"] == "shipped":
            drift.append(f"{where} is 'shipped' but comes after the current release '{data['currentSlug']}'. Fix its status in {EDIT_HINT}, or move currentSlug once it is published.")
        for item in rel.get("items", []):
            at = f"{where}, item \"{item['text']}\""
            status = item["status"]
            if future and status == "shipped":
                drift.append(f"{at} is 'shipped' in a release after the current one '{data['currentSlug']}'. Shipped means in a published release; fix its status in {EDIT_HINT}.")
            linked = item.get("stints", [])
            if not linked:
                continue
            missing = [s for s in linked if s not in stints]
            for sid in missing:
                drift.append(f"{at} links stint {sid}, which does not exist. Fix its stints in {EDIT_HINT}.")
            known = [stints[s] for s in linked if s in stints]
            for s in known:
                if s.state == "archived":
                    drift.append(f"{at} links {s.label()}, which is archived. Remove it or link its successor in {EDIT_HINT}.")
            open_ = [s for s in known if s.state not in ("done", "archived")]
            if status == "shipped":
                for s in open_:
                    drift.append(f"{at} is 'shipped' but {s.label()} is {s.state}. Set the item to 'in progress' in {EDIT_HINT}, or finish the stint.")
                continue
            if missing or open_ or any(s.state == "archived" for s in known):
                continue
            if status == "planned":
                drift.append(f"{at} is 'planned' but every linked stint is done: {', '.join(s.label() for s in known)}. Set its status to 'in progress' in {EDIT_HINT}; it becomes 'shipped' only once a published release carries the work.")
            unnamed = [s for s in known if s.id not in named]
            if unnamed:
                info.append(f"{at}: all linked stints are done, but no commit on {ref} names {', '.join(s.label() for s in unnamed)}; confirm the work is in this release before marking it shipped.")
            else:
                info.append(f"{at}: eligible. All linked stints are done and on {ref}; mark it 'shipped' in {EDIT_HINT} once the release cut from {ref} is published.")
    return drift, info


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--roadmap-ref", help="read the roadmap committed on this ref; promotion passes origin/alpha, the published copy (default: working tree)")
    parser.add_argument("--commits-ref", help="the release candidate whose commits are searched (default: HEAD)")
    parser.add_argument("--roadmap", type=Path, help="read this roadmap file instead (tests)")
    args = parser.parse_args(argv)
    ref = args.commits_ref or "HEAD"
    try:
        if args.roadmap is not None:
            source = f"file {args.roadmap}"
            data = load_roadmap(args.roadmap)
        elif args.roadmap_ref:
            source = f"committed {args.roadmap_ref}:{EDIT_HINT}"
            data = load_roadmap_at(args.roadmap_ref)
        else:
            source = f"working tree {EDIT_HINT}"
            data = load_roadmap(DEFAULT_ROADMAP)
        stints = load_stints()
        named = stints_named_on(ref)
    except CheckError as e:
        print(f"check-roadmap: error: {e}", file=sys.stderr)
        return 2
    drift, info = check(data, stints, named, ref)
    linked = sum(1 for r in data["releases"] for i in r.get("items", []) if i.get("stints"))
    total = sum(len(r.get("items", [])) for r in data["releases"])
    print(f"check-roadmap: roadmap from {source}; commits from {ref}; {linked} of {total} items linked to stints")
    for line in info:
        print(f"  info: {line}")
    for line in drift:
        print(f"  DRIFT: {line}")
    if drift:
        sys.stdout.flush()
        print(f"check-roadmap: {len(drift)} drift problem(s). The public roadmap disagrees with the stints; fix {EDIT_HINT} or the stints, then rerun `just check-roadmap`.", file=sys.stderr)
        return 1
    print("check-roadmap: no drift")
    return 0


if __name__ == "__main__":
    sys.exit(main())
