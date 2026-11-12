#!/usr/bin/env python3
"""Verify the signer facts recorded in an extracted macOS release archive."""
import argparse
import json
import subprocess
from pathlib import Path
import sys
import tarfile
import tempfile

from package import macos_signature


def main(args):
    archive = args.archive.resolve()
    with tempfile.TemporaryDirectory(prefix="plexi-signing-archive-") as temporary:
        root = Path(temporary)
        with tarfile.open(archive, "r:gz") as payload:
            for member in payload.getmembers():
                destination = root / member.name
                if destination.resolve().parent != root.resolve() and root.resolve() not in destination.resolve().parents:
                    raise AssertionError(f"unsafe archive member {member.name!r}")
                if member.issym() or member.islnk() or not (member.isdir() or member.isfile()):
                    raise AssertionError(f"unsupported archive member {member.name!r}")
                payload.extract(member, root)
        manifest = json.loads((root / "package.json").read_text())
        signing = manifest.get("macos_signing")
        if not signing:
            raise AssertionError("macOS package manifest does not record its signing identity")
        apps = list(root.glob("*.app"))
        if len(apps) != 1:
            raise AssertionError(f"expected exactly one macOS bundle, found {apps}")
        subprocess.run(["codesign", "--verify", "--deep", "--strict", str(apps[0])], check=True)
        actual = macos_signature(apps[0])
        if actual != signing:
            raise AssertionError(f"archive signer drifted from manifest: expected={signing}, actual={actual}")
        if args.expected_certificate_sha1:
            expected = args.expected_certificate_sha1.upper().replace(":", "")
            if actual["certificate_sha1"] != expected:
                raise AssertionError("archive certificate differs from the protected release identity")
        if args.expected_authority and args.expected_authority != actual["authorities"][0]:
            raise AssertionError(f"archive does not carry expected authority {args.expected_authority!r}")
        suffix = "" if manifest["channel"] == "stable" else "-" + manifest["channel"]
        if not actual["designated_requirement"].startswith(f'identifier "com.ianjamesburke.plexi{suffix}" and '):
            raise AssertionError("archive designated requirement is not bound to the Plexi bundle identifier")
        print(
            "PASS: macOS archive signer "
            f"authority={actual['authorities'][0]!r} certificate_sha1={actual['certificate_sha1']}"
        )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--expected-certificate-sha1")
    parser.add_argument("--expected-authority")
    main(parser.parse_args())
