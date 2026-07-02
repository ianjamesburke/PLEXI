#!/usr/bin/env python3
"""Drift gate for the Plexi app-authoring guidance.

Guards the authoring docs the same way check-sdk-docs guards the generated SDK
reference: doc rot becomes a CI failure. Two checks, both against live source:

1. Effect/component name drift. Ground truth is parsed (ast, no imports) from
   sdk/python/plexi_sdk/effects.py and ui.py. Every effect/component name that
   the canonical guide (sdk/python/AUTHORING.md, inside <!-- drift-check:* -->
   markers) or the `plexi app init --help` block (src/cli/args.rs) names must
   exist in the SDK. Catches the exact rot fixed in stint 0332 (LogInfo/LogWarn/
   LogError and HttpRequest in help text; TextInput vs TextEdit).

2. Dead relative links. Every relative markdown link in the authoring-path docs
   must resolve to a file that exists (catches the dead SDK_QUICKSTART.md link).

Run via: just check-authoring-docs
"""
from __future__ import annotations

import ast
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SDK_PKG = REPO_ROOT / "sdk" / "python" / "plexi_sdk"
EFFECTS_PY = SDK_PKG / "effects.py"
UI_PY = SDK_PKG / "ui.py"
AUTHORING_MD = REPO_ROOT / "sdk" / "python" / "AUTHORING.md"
ARGS_RS = REPO_ROOT / "src" / "cli" / "args.rs"

# Docs whose relative links must resolve. The authoring path an agent follows.
LINK_CHECKED_DOCS = [
    REPO_ROOT / "sdk" / "python" / "AUTHORING.md",
    REPO_ROOT / "sdk" / "python" / "README.md",
    REPO_ROOT / "sdk" / "python" / "AGENTS.md",
    REPO_ROOT / "apps" / "AGENTS.md",
]

IDENT = re.compile(r"^[A-Z][A-Za-z0-9]+$")


def _public_top_level(path: Path) -> set[str]:
    """Public top-level class/function names in a module (ast, no import)."""
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    names: set[str] = set()
    for node in tree.body:
        if isinstance(node, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
            if not node.name.startswith("_"):
                names.add(node.name)
        elif isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name) and target.id == "__all__":
                    if isinstance(node.value, (ast.List, ast.Tuple)):
                        for elt in node.value.elts:
                            if isinstance(elt, ast.Constant) and isinstance(elt.value, str):
                                names.add(elt.value)
    return names


def _marker_block(text: str, tag: str) -> str:
    """Text between <!-- drift-check:TAG --> and <!-- /drift-check:TAG -->."""
    m = re.search(
        rf"<!--\s*drift-check:{tag}\s*-->(.*?)<!--\s*/drift-check:{tag}\s*-->",
        text,
        re.DOTALL,
    )
    if not m:
        raise SystemExit(
            f"ERROR: {AUTHORING_MD.relative_to(REPO_ROOT)} is missing the "
            f"<!-- drift-check:{tag} --> marker block."
        )
    return m.group(1)


def _backticked_idents(block: str) -> set[str]:
    return {t for t in re.findall(r"`([A-Za-z0-9]+)`", block) if IDENT.match(t)}


def _help_block(rs: str) -> str:
    """The after_long_help raw-string body in args.rs (text, not parsed Rust)."""
    m = re.search(r'APP DEVELOPMENT GUIDE:(.*?)"#', rs, re.DOTALL)
    if not m:
        raise SystemExit("ERROR: could not locate the APP DEVELOPMENT GUIDE help block in args.rs.")
    return m.group(1)


def _help_region(help_text: str, start: str, end_pred) -> set[str]:
    """CapWords tokens on the list lines after `start` up to end_pred(line)."""
    lines = help_text.splitlines()
    out: set[str] = set()
    collecting = False
    for line in lines:
        if start in line:
            collecting = True
            continue
        if collecting:
            if end_pred(line):
                break
            for tok in re.split(r"[,\s/]+", line.strip()):
                if IDENT.match(tok):
                    out.add(tok)
    return out


def check_names() -> list[str]:
    errors: list[str] = []
    effects = _public_top_level(EFFECTS_PY)
    components = _public_top_level(UI_PY)

    authoring = AUTHORING_MD.read_text(encoding="utf-8")
    doc_effects = _backticked_idents(_marker_block(authoring, "effects"))
    doc_components = _backticked_idents(_marker_block(authoring, "components"))

    help_text = _help_block(ARGS_RS.read_text(encoding="utf-8"))
    help_components = _help_region(
        help_text, "Key widgets:", lambda ln: ln.strip() == ""
    )
    help_effects = _help_region(
        help_text, "Effects:", lambda ln: "Logging is not an effect" in ln
    )

    for name in sorted(doc_effects | help_effects):
        if name not in effects:
            where = "AUTHORING.md" if name in doc_effects else "args.rs help"
            errors.append(
                f"effect `{name}` in {where} does not exist in effects.py"
            )
    for name in sorted(doc_components | help_components):
        if name not in components:
            where = "AUTHORING.md" if name in doc_components else "args.rs help"
            errors.append(
                f"component `{name}` in {where} does not exist in ui.py"
            )
    return errors


LINK_RE = re.compile(r"\[[^\]]*\]\(([^)]+)\)")


def check_links() -> list[str]:
    errors: list[str] = []
    for doc in LINK_CHECKED_DOCS:
        text = doc.read_text(encoding="utf-8")
        for target in LINK_RE.findall(text):
            target = target.strip()
            if target.startswith(("http://", "https://", "#", "mailto:")):
                continue
            path_part = target.split("#", 1)[0]
            if not path_part:
                continue
            resolved = (doc.parent / path_part).resolve()
            if not resolved.exists():
                errors.append(
                    f"{doc.relative_to(REPO_ROOT)} links to missing "
                    f"`{path_part}`"
                )
    return errors


def main() -> None:
    errors = check_names() + check_links()
    if errors:
        print("Authoring-doc drift detected:", file=sys.stderr)
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        print(
            "\nFix the doc/help text or the SDK source so they agree.",
            file=sys.stderr,
        )
        sys.exit(1)
    print("Authoring docs are consistent with the SDK.")


if __name__ == "__main__":
    main()
