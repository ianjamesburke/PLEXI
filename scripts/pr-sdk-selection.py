#!/usr/bin/env python3
"""Conservative source SDK smoke selection; installed SDK guard is independent."""
import subprocess
import sys


def selection(paths):
    if not paths:
        return True, "empty or unknown diff"
    # Only these native-only surfaces are proven unrelated. Everything else,
    # including protocol/runtime/build inputs and unknown paths, keeps smoke.
    native = ("src/overlays/", "src/ui/", "src/editor/", "src/config/",
              "src/host/context", "src/host/model", "src/host/navigation",
              "src/workspace/context")
    for path in paths:
        if path.startswith(native) or path == "src/app/text_editor_app.rs":
            continue
        if path.endswith(".md") and path.startswith(("docs/", ".agents/", ".claude/")):
            continue
        return True, f"affected or unknown boundary: {path}"
    return False, "native UI/context or workflow documentation only"


def main():
    try:
        changed = subprocess.check_output(
            ["git", "diff", "--name-only", "--no-renames", "-z", f"{sys.argv[1]}...HEAD"])
        dirty = subprocess.check_output(["git", "status", "--porcelain", "-z"])
        if dirty:
            print("run: dirty source requires conservative SDK smoke")
        else:
            run, reason = selection(changed.decode().rstrip("\0").split("\0") if changed else [])
            print(f"{'run' if run else 'skip'}: {reason}")
    except (OSError, subprocess.CalledProcessError, UnicodeError) as error:
        print(f"run: cannot establish affected SDK boundary: {error}")


if __name__ == "__main__":
    main()
