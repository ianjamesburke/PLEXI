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
commit message on REF, so they ship with the release cut from REF. Mark them
"shipped" in roadmap.json once that release is published.

Errors (exit 2): no `stint` on PATH, no .stint/ above the repo, unreadable
roadmap, unknown REF.

Usage: check_roadmap.py [--ref REF] [--roadmap PATH]
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

REPO = Path(__file__).resolve().parent.parent
DEFAULT_ROADMAP = REPO / "website/src/lib/roadmap.json"
EDIT_HINT = "website/src/lib/roadmap.json"
ITEM_STATUSES = {"shipped", "in progress", "planned"}
RELEASE_STATUSES = {"shipped", "in progress", "next", "planned", "after 1.0"}
STINT_ID = re.compile(r"^\d{4}$")
# Commit messages are free text. A stint is named on a line that says
# "stint" or "stints" followed by its four-digit id; "#1234" is a PR number.
STINT_LINE = re.compile(r"\bstints?\b", re.IGNORECASE)
ID_TOKEN = re.compile(r"(?<![#\d])\d{4}(?!\d)")


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
        data = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as e:
        raise CheckError(f"cannot read roadmap {path}: {e}") from e
    releases = data.get("releases")
    if not isinstance(releases, list) or not isinstance(data.get("currentSlug"), str):
        raise CheckError(f"{path}: needs a 'currentSlug' string and a 'releases' list")
    slugs = [r.get("slug") for r in releases]
    if data["currentSlug"] not in slugs:
        raise CheckError(f"{path}: currentSlug '{data['currentSlug']}' names no release")
    for r in releases:
        if r.get("status") not in RELEASE_STATUSES:
            raise CheckError(f"{path}: release '{r.get('slug')}' has unknown status {r.get('status')!r}")
        for item in r.get("items", []):
            if item.get("status") not in ITEM_STATUSES:
                raise CheckError(f"{path}: item {item.get('text')!r} has unknown status {item.get('status')!r}")
            for sid in item.get("stints", []):
                if not isinstance(sid, str) or not STINT_ID.match(sid):
                    raise CheckError(f"{path}: item {item.get('text')!r} lists stint {sid!r}; ids are four-digit strings like \"0577\"")
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
    try:
        rows = json.loads(proc.stdout)
    except json.JSONDecodeError as e:
        raise CheckError(f"`stint list --all --json` printed invalid JSON: {e}") from e
    return {row["id"]: Stint(row["id"], row["state"], row["title"]) for row in rows}


def stints_named_on(ref: str, repo: Path = REPO) -> set[str]:
    try:
        proc = subprocess.run(["git", "-C", str(repo), "log", "--format=%B%x00", ref, "--"], capture_output=True, text=True, check=False)
    except OSError as e:
        raise CheckError(f"`git log {ref}` failed to start: {e}") from e
    if proc.returncode != 0:
        raise CheckError(f"`git log {ref}` exited {proc.returncode}: {proc.stderr.strip()} (fetch the ref or pass --ref)")
    named: set[str] = set()
    for line in proc.stdout.splitlines():
        if STINT_LINE.search(line):
            named.update(ID_TOKEN.findall(line))
    return named


def check(data: dict, stints: dict[str, Stint], named: set[str], ref: str) -> tuple[list[str], list[str]]:
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
                drift.append(f"{at} is 'planned' but every linked stint is done: {', '.join(s.label() for s in known)}. Set it to at least 'in progress' in {EDIT_HINT}.")
            unnamed = [s for s in known if s.id not in named]
            if unnamed:
                info.append(f"{at}: all linked stints are done, but no commit on {ref} names {', '.join(s.label() for s in unnamed)}; confirm the work is in this release before marking it shipped.")
            else:
                info.append(f"{at}: eligible. All linked stints are done and on {ref}; mark it 'shipped' in {EDIT_HINT} once the release cut from {ref} is published.")
    return drift, info


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--ref", default="origin/alpha", help="the commit the next release is cut from (default: origin/alpha)")
    parser.add_argument("--roadmap", type=Path, default=DEFAULT_ROADMAP)
    args = parser.parse_args(argv)
    try:
        data = load_roadmap(args.roadmap)
        stints = load_stints()
        named = stints_named_on(args.ref)
    except CheckError as e:
        print(f"check-roadmap: error: {e}", file=sys.stderr)
        return 2
    drift, info = check(data, stints, named, args.ref)
    linked = sum(1 for r in data["releases"] for i in r.get("items", []) if i.get("stints"))
    total = sum(len(r.get("items", [])) for r in data["releases"])
    print(f"check-roadmap: {linked} of {total} items linked to stints; release candidate {args.ref}")
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
