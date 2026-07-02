#!/usr/bin/env python3
"""Package the public apps and regenerate the hosted registry index.

Single source of truth for `website/public/registry/v1/`: it walks the `apps/`
tree, packages every app that opts into the marketplace (its `manifest.toml`
declares `[marketplace] visibility = "public"`), checksum-addresses each
`.plexipkg`, and writes a deterministic `index.json`. The browse pages under
`website/src/pages/apps/` import that same index, so the catalog a first user
sees and the artifacts the host installs never drift.

Republishing after an app fix is one command: `just website-registry`.

The Core apps are operator-published — reviewed by construction — so every
listing carries `reviewed_native = true` and the "Reviewed native process"
trust label. An app without a `[marketplace]` section is skipped: `github-issues`
(shells out to `git`/`gh`) and `stats` (reads `PLEXI_SOCKET` directly) both stay
out of the catalog until those host-routing patterns are host-mediated or the
app moves to the sandboxed wasm container. Only apps that package cleanly under
the standard bypass scan are published.
"""
from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
APPS_DIR = REPO_ROOT / "apps"
REGISTRY_DIR = REPO_ROOT / "website" / "public" / "registry" / "v1"
PACKAGES_DIR = REGISTRY_DIR / "packages"
INDEX_PATH = REGISTRY_DIR / "index.json"

SCHEMA_VERSION = 1
# Mirrors TrustLabel::ReviewedNative::display_str() in src/app/package.rs.
TRUST_LABEL = "Reviewed native process — human-reviewed; not sandboxed"

PLEXI_BIN = os.environ.get("PLEXI_BIN", str(REPO_ROOT / "target" / "debug" / "plexi"))


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 16), b""):
            h.update(chunk)
    return h.hexdigest()


def package_app(app_dir: Path, out: Path) -> None:
    """Build a `.plexipkg` for one app dir (fail-closed)."""
    env = {**os.environ, "PLEXI_CHANNEL": os.environ.get("PLEXI_CHANNEL", "alpha")}
    result = subprocess.run(
        [PLEXI_BIN, "app", "package", str(app_dir), "--out", str(out)],
        capture_output=True,
        text=True,
        env=env,
    )
    if result.returncode != 0 or not out.exists():
        raise SystemExit(
            f"package failed for {app_dir.name}:\n{result.stdout}\n{result.stderr}"
        )


def entry_for(app_dir: Path, checksum: str) -> dict:
    manifest = tomllib.loads((app_dir / "manifest.toml").read_text())
    app = manifest["app"]
    market = manifest["marketplace"]
    caps = manifest.get("app", {}).get("capabilities", {}).get("capabilities", [])
    return {
        "id": app["id"],
        "name": app["name"],
        "version": app["version"],
        "description": app.get("description", ""),
        "publisher": market.get("publisher", "plexi"),
        "capabilities": caps,
        "tags": ["core", "free", "reviewed-native"],
        "trust_label": TRUST_LABEL,
        "reviewed_native": True,
        "visibility": market.get("visibility", "public"),
        "price": market.get("price", "free"),
        "checksum": checksum,
    }


def is_public(app_dir: Path) -> bool:
    manifest_path = app_dir / "manifest.toml"
    if not manifest_path.is_file():
        return False
    manifest = tomllib.loads(manifest_path.read_text())
    return manifest.get("marketplace", {}).get("visibility") == "public"


def main() -> None:
    if not Path(PLEXI_BIN).exists():
        raise SystemExit(
            f"plexi binary not found at {PLEXI_BIN}; run `cargo build --bin plexi` "
            "or set PLEXI_BIN"
        )

    public_apps = sorted(
        (d for d in APPS_DIR.iterdir() if d.is_dir() and is_public(d)),
        key=lambda d: d.name,
    )
    if not public_apps:
        raise SystemExit(f"no public apps found under {APPS_DIR}")

    # Rebuild the artifact store from scratch so a delisted app leaves no
    # orphaned package behind.
    if PACKAGES_DIR.exists():
        shutil.rmtree(PACKAGES_DIR)
    PACKAGES_DIR.mkdir(parents=True)

    entries = []
    with tempfile.TemporaryDirectory() as tmp:
        for app_dir in public_apps:
            pkg = Path(tmp) / f"{app_dir.name}.plexipkg"
            package_app(app_dir, pkg)
            checksum = sha256(pkg)
            shutil.copy2(pkg, PACKAGES_DIR / f"{checksum}.plexipkg")
            entry = entry_for(app_dir, checksum)
            entries.append(entry)
            print(f"published {entry['id']} v{entry['version']} -> {checksum}")

    entries.sort(key=lambda e: e["id"])
    index = {"schema_version": SCHEMA_VERSION, "apps": entries}
    INDEX_PATH.write_text(json.dumps(index, indent=2, ensure_ascii=False) + "\n")
    print(f"wrote {INDEX_PATH.relative_to(REPO_ROOT)} with {len(entries)} apps")


if __name__ == "__main__":
    main()
