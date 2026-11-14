#!/usr/bin/env python3
"""List ignored lane content that is not an explicitly disposable artifact."""
import os
from pathlib import Path, PurePosixPath
import subprocess
import sys


def disposable(root: Path, entry: str) -> bool:
    path = PurePosixPath(entry.rstrip('/'))
    parts = path.parts
    allowed = (
        parts in {('target',), ('assets', 'python'), ('scripts', '__pycache__'), ('sdk', 'python', '.venv'),
                  ('sdk', 'python', '.pytest_cache')}
        or (len(parts) >= 3 and parts[0] in {'plugins', 'sdk'} and parts[-1] == '__pycache__')
    )
    if not allowed or path.is_absolute() or '..' in parts:
        return False
    # Refuse paths redirected through symlinks, including the artifact itself.
    current = root
    for part in parts:
        current = current / part
        if current.is_symlink():
            return False
    return current.is_dir()


def main() -> None:
    root = Path(sys.argv[1])
    result = subprocess.run(
        ['git', '-C', os.fspath(root), 'ls-files', '--others', '--ignored',
         '--exclude-standard', '--directory', '--no-empty-directory', '-z'],
        check=True, stdout=subprocess.PIPE,
    )
    for raw in result.stdout.split(b'\0'):
        if not raw:
            continue
        path = os.fsdecode(raw)
        if disposable(root, path):
            print(f'cleanup: recognized generated artifact: {path}', file=sys.stderr)
        else:
            print(path)


if __name__ == '__main__':
    main()
