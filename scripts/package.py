#!/usr/bin/env python3
"""Assemble the same relocatable package for source installs and releases."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request
import zipfile

REPO = Path(__file__).resolve().parent.parent
WASI_VERSION = "3.12.12"
WASI_SHA = "e40dac3ae68c988b9dcbf2ff6a1fb1b84435aa05b20defcd155801339f35feb2"
STDLIB_SHA = "487c908ddf4097a1b9ba859f25fe46d22ccaabfb335880faac305ac62bffb79b"
MODULE_SHA = "62392f07fee032c22e3aa84be033c07105cd42424e5149058b9f5449a8deb272"


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def download(url, target, expected):
    with urllib.request.urlopen(url, timeout=120) as response, target.open("wb") as output:
        shutil.copyfileobj(response, output)
    if digest(target) != expected:
        raise ValueError(f"SHA-256 mismatch: {url}")


def runtime(cache):
    target = cache / f"cpython-{WASI_VERSION}"
    if (target / "python.wasm").is_file() and digest(target / "python.wasm") == MODULE_SHA and (target / "Lib/encodings/__init__.py").is_file():
        return target
    cache.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=cache) as temporary:
        stage = Path(temporary)
        download(f"https://github.com/brettcannon/cpython-wasi-build/releases/download/v{WASI_VERSION}/python-{WASI_VERSION}-wasi_sdk-20.zip", stage / "wasi.zip", WASI_SHA)
        download(f"https://www.python.org/ftp/python/{WASI_VERSION}/Python-{WASI_VERSION}.tgz", stage / "stdlib.tgz", STDLIB_SHA)
        payload = stage / "payload"
        payload.mkdir()
        with zipfile.ZipFile(stage / "wasi.zip") as archive:
            (payload / "python.wasm").write_bytes(archive.read("python.wasm"))
        if digest(payload / "python.wasm") != MODULE_SHA:
            raise ValueError("WASI interpreter checksum mismatch")
        with tarfile.open(stage / "stdlib.tgz") as archive:
            prefix = f"Python-{WASI_VERSION}/Lib/"
            for member in archive:
                if member.isfile() and member.name.startswith(prefix):
                    name = Path(member.name.removeprefix(prefix))
                    if ".." in name.parts or name.is_absolute():
                        raise ValueError("unsafe stdlib member")
                    destination = payload / "Lib" / name
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    with archive.extractfile(member) as source, destination.open("wb") as output:
                        shutil.copyfileobj(source, output)
        if target.exists():
            shutil.rmtree(target)
        payload.rename(target)
    return target


def display_name(channel):
    if channel == "stable":
        return "Plexi"
    if channel.startswith("pr-"):
        return "Plexi PR" + channel[3:]
    return "Plexi " + channel.capitalize()


def assemble(args):
    channel = "stable" if args.channel == "main" else args.channel
    if not channel or any(c not in "abcdefghijklmnopqrstuvwxyz0123456789-" for c in channel):
        raise ValueError("invalid channel")
    name = "plexi" if channel == "stable" else "plexi-" + channel
    identity = json.loads(subprocess.check_output([str(args.binary.resolve()), "--build-info"], text=True))
    if args.tag and args.tag != identity["tag"]:
        raise ValueError(f"binary was built for {identity['tag']}, requested {args.tag}; rebuild with PLEXI_BUILD_TAG")
    args.output.mkdir(parents=True, exist_ok=True)
    root = args.output / f"package-{args.platform}-{channel}"
    if root.exists():
        raise ValueError(f"package destination already exists: {root}")
    root.mkdir()
    if args.platform.startswith("macos-"):
        bundle = root / (display_name(channel) + ".app")
        executable = bundle / "Contents/MacOS" / name
        resources = bundle / "Contents/Resources"
        executable.parent.mkdir(parents=True)
        resources.mkdir(parents=True)
        info = {
            "CFBundleName": display_name(channel), "CFBundleDisplayName": display_name(channel),
            "CFBundleIdentifier": "com.ianjamesburke.plexi" + ("" if channel == "stable" else "-" + channel),
            "CFBundleExecutable": name, "CFBundlePackageType": "APPL", "CFBundleInfoDictionaryVersion": "6.0",
            "CFBundleShortVersionString": identity["version"], "CFBundleVersion": identity["version"],
            "CFBundleIconFile": "app-icon.icns", "LSMinimumSystemVersion": "12.0", "NSHighResolutionCapable": True,
            "NSMicrophoneUsageDescription": "Plexi uses the microphone for audio input.",
        }
        fragment = (REPO / "assets/Info.plist.fragment").read_text()
        info.update(plistlib.loads(("<?xml version='1.0' encoding='UTF-8'?><plist version='1.0'><dict>" + fragment + "</dict></plist>").encode()))
        (bundle / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
        shutil.copy2(REPO / "assets/app-icon.icns", resources)
    else:
        executable = root / (name + (".exe" if args.platform.startswith("windows-") else ""))
        resources = root / "resources"
        resources.mkdir()
    shutil.copy2(args.binary, executable)
    executable.chmod(0o755)
    installer_name = "plexi-installer.exe" if args.platform.startswith("windows-") else "plexi-installer"
    shutil.copy2(args.installer, root / installer_name)
    (root / installer_name).chmod(0o755)
    ignore = shutil.ignore_patterns("__pycache__", "*.pyc", ".venv", ".DS_Store")
    shutil.copytree(REPO / "sdk/python/plexi_sdk", resources / "sdk/plexi_sdk", ignore=ignore)
    shutil.copy2(REPO / "sdk/python/pyproject.toml", resources / "sdk/pyproject.toml")
    shutil.copytree(runtime(args.runtime_cache), resources / "wasm-bundles" / f"cpython-{WASI_VERSION}", ignore=ignore)
    shutil.copytree(REPO / "apps/calc", resources / "smoke-app", ignore=ignore)
    shutil.copy2(REPO / "assets/app-icon.png", resources)
    shutil.copy2(REPO / "scripts/default-config.toml", resources / "default-config.toml")
    for source, destination in [("agents", "agents"), ("skills", "skills"), ("scripts/default-scripts", "scripts")]:
        shutil.copytree(REPO / source, resources / destination, ignore=ignore)
    if channel == "alpha" or channel.startswith("pr-"):
        for manifest in (REPO / "apps").glob("*/manifest.toml"):
            shutil.copytree(manifest.parent, resources / "maintained-apps" / manifest.parent.name, ignore=ignore)

    if args.platform.startswith("macos-"):
        # All channel mutations precede signing. Stable archives consumed by
        # alpha/beta carry separately assembled variants; installers never patch them.
        identity_name = os.environ.get("PLEXI_SIGN_IDENTITY", "-")
        subprocess.run(["codesign", "--force", "--deep", "--sign", identity_name, str(bundle)], check=True)
        subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)
    manifest = dict(identity, schema=1, channel=channel, platform=args.platform,
                    executable=executable.relative_to(root).as_posix(), resources=resources.relative_to(root).as_posix(),
                    files={path.relative_to(root).as_posix(): digest(path) for path in sorted(root.rglob("*")) if path.is_file()})
    (root / "package.json").write_text(json.dumps(manifest, indent=2) + "\n")
    suffix = "" if channel == "stable" else "-" + channel
    archive = args.output / f"plexi-{args.platform}{suffix}.{'zip' if args.platform.startswith('windows-') else 'tar.gz'}"
    if args.platform.startswith("windows-"):
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as output:
            for path in sorted(root.rglob("*")):
                if path.is_file():
                    output.write(path, path.relative_to(root).as_posix())
    else:
        with tarfile.open(archive, "w:gz", format=tarfile.PAX_FORMAT) as output:
            for path in sorted(root.iterdir()):
                output.add(path, path.name)
    archive.with_name(archive.name + ".sha256").write_text(f"{digest(archive)}  {archive.name}\n")
    print(root)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--installer", required=True, type=Path)
    parser.add_argument("--platform", required=True, choices=["macos-arm64", "macos-x64", "linux-x64", "windows-x64"])
    parser.add_argument("--channel", default="stable")
    parser.add_argument("--tag")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--runtime-cache", type=Path, default=REPO / "target/distribution-runtime")
    assemble(parser.parse_args())
