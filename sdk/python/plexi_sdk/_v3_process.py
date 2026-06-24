"""Entry point for v3 apps: python -m plexi_sdk._v3_process <app.py> [args...]

Delegates to V3AppRuntime. This module exists solely as the entry point
the host invokes; the runtime implementation lives in _v3_runtime.
"""
from __future__ import annotations


def main(argv: list[str] | None = None) -> None:
    from ._v3_runtime import main as _runtime_main
    _runtime_main(argv)


if __name__ == "__main__":
    main()
