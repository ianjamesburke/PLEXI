"""Version 2 package migration. Parse source without importing or executing it."""
import ast
import io
import os
from pathlib import Path
import shutil
import sys
import tokenize

source, target = map(Path, sys.argv[1:])
shutil.copytree(source, target, symlinks=True, ignore=shutil.ignore_patterns(".venv", "__pycache__"))
names = {
    "AppHarness": "PluginHarness", "AppBar": "SurfaceHeader",
    "AppPalette": "PluginPalette", "AppEvent": "PluginEvent",
    "app_id": "plugin_id", "_app_id": "_plugin_id",
    "V3AppRuntime": "PluginRuntime", "app_path": "plugin_path",
}
for root, dirs, files in os.walk(target, followlinks=False):
    dirs[:] = [d for d in dirs if d not in {".git", ".venv", "__pycache__"}]
    for filename in files:
        path = Path(root) / filename
        if path.suffix != ".py":
            continue
        if path.is_symlink():
            raise ValueError(f"cannot migrate symlinked Python source: {path}")
        with tokenize.open(path) as stream:
            text = stream.read()
            encoding = stream.encoding
        tree = ast.parse(text, filename=str(path))
        lines = text.splitlines(keepends=True)
        actors = set()
        for node in ast.walk(tree):
            values = []
            if isinstance(node, ast.Call):
                values = [keyword.value for keyword in node.keywords if keyword.arg == "actor"]
            elif isinstance(node, ast.Dict):
                values = [value for key, value in zip(node.keys, node.values)
                          if isinstance(key, ast.Constant) and key.value == "actor"]
            for value in values:
                if isinstance(value, ast.Constant) and value.value == "app":
                    # AST columns count UTF-8 bytes; tokenize columns count characters.
                    column = len(lines[value.lineno - 1].encode()[:value.col_offset].decode())
                    actors.add((value.lineno, column))
        offsets = [0]
        for line in lines:
            offsets.append(offsets[-1] + len(line))
        edits = []
        for token in tokenize.generate_tokens(io.StringIO(text).readline):
            replacement = token.string
            if token.type == tokenize.NAME:
                replacement = names.get(token.string, token.string)
            elif token.type == tokenize.STRING:
                try:
                    value = ast.literal_eval(token.string)
                except (ValueError, SyntaxError):
                    continue
                if token.start in actors:
                    replacement = repr("plugin")
                elif isinstance(value, str) and value in {"app_id", "app_event", "AppBar", "app_bar", "pgap/3", "spawn.app"}:
                    replacement = repr({"app_id": "plugin_id", "app_event": "plugin_event", "AppBar": "SurfaceHeader", "app_bar": "surface_header", "pgap/3": "pgap/4", "spawn.app": "spawn.plugin"}[value])
            if replacement != token.string:
                start = offsets[token.start[0] - 1] + token.start[1]
                end = offsets[token.end[0] - 1] + token.end[1]
                edits.append((start, end, replacement))
        for start, end, replacement in reversed(edits):
            text = text[:start] + replacement + text[end:]
        if edits:
            ast.parse(text, filename=str(path))
            path.write_text(text, encoding=encoding)
