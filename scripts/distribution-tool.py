#!/usr/bin/env python3
"""Run the authoritative native installer from a source/digest-verified cache."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def identity():
    paths = [ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', Path(__file__)]
    paths += sorted((ROOT / 'crates/distribution').rglob('*.rs'))
    paths += [ROOT / 'crates/distribution/Cargo.toml']
    config = ROOT / '.cargo/config.toml'
    if config.is_file():
        paths.append(config)
    config_directories = [parent / '.cargo' for parent in [*reversed(ROOT.parents), ROOT]]
    config_directories.append(Path(os.environ.get('CARGO_HOME', str(Path.home() / '.cargo'))))
    configs = {str(directory / name): digest(directory / name)
               for directory in config_directories for name in ('config', 'config.toml')
               if (directory / name).is_file()}
    return {'configs': configs, 'schema': 1, 'sources': {str(p.relative_to(ROOT)): digest(p) for p in paths},
            'platform': [platform.system(), platform.machine()],
            'toolchain': subprocess.check_output(['rustc', '-vV'], text=True),
            'flags': {k: v for k, v in os.environ.items() if k.startswith(('RUST', 'CARGO_')) and k != 'CARGO_TARGET_DIR' or k in ('CC', 'CXX', 'SDKROOT', 'DEVELOPER_DIR', 'MACOSX_DEPLOYMENT_TARGET')},
            'cargo_home_config': {str(p): digest(p) for p in (Path(os.environ.get('CARGO_HOME', str(Path.home() / '.cargo'))) / n for n in ('config', 'config.toml')) if p.is_file()}}


def prepare():
    key = identity()
    common = subprocess.check_output(['git', 'rev-parse', '--path-format=absolute', '--git-common-dir'], cwd=ROOT, text=True).strip()
    name = hashlib.sha256(common.encode()).hexdigest()[:16]
    identity_sha = hashlib.sha256(json.dumps(key, sort_keys=True).encode()).hexdigest()
    directory = Path(os.environ.get('PLEXI_DISTRIBUTION_CACHE_ROOT', str(Path.home() / '.cache/plexi-distribution'))) / name / identity_sha
    directory.mkdir(parents=True, exist_ok=True)
    binary = directory / 'plexi-installer'
    receipt = directory / 'evidence.json'
    def valid():
        try:
            value = json.loads(receipt.read_text())
            return value['identity'] == key and binary.is_file() and not binary.is_symlink() and value['sha256'] == digest(binary)
        except (OSError, ValueError, KeyError):
            return False
    if valid():
        return binary
    # The small native-tool cache is separate from host target slots and survives
    # worktree retirement. Warm calls never acquire Cargo's global memory lease.
    with (directory / 'lock').open('a+') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if valid():
            return binary
        print('distribution-tool: cache miss; building authoritative installer under Cargo lease', file=sys.stderr)
        result = subprocess.run(['bash', str(ROOT / 'scripts/cargo-with-lease.sh'), 'cargo', 'build',
                                 '--manifest-path', str(ROOT / 'Cargo.toml'), '--release', '-p',
                                 'plexi-distribution', '--bin', 'plexi-installer',
                                 '--message-format=json-render-diagnostics'], cwd=ROOT,
                                stdout=subprocess.PIPE, text=True, check=True)
        artifacts = [json.loads(line) for line in result.stdout.splitlines() if line.startswith('{')]
        executables = {message['executable'] for message in artifacts
                       if message.get('reason') == 'compiler-artifact'
                       and message.get('target', {}).get('name') == 'plexi-installer'
                       and message.get('executable')}
        if len(executables) != 1:
            raise ValueError('Cargo did not identify exactly one installer executable')
        source = Path(executables.pop())
        subprocess.run([str(source), '--help'], stdout=subprocess.DEVNULL, check=True)
        with tempfile.TemporaryDirectory(dir=directory) as temporary:
            copied = Path(temporary) / 'installer'
            shutil.copy2(source, copied)
            manifest = Path(temporary) / 'evidence'
            manifest.write_text(json.dumps({'identity': key, 'sha256': digest(copied)}, sort_keys=True))
            if identity() != key:
                raise ValueError('installer inputs changed during build')
            os.replace(copied, binary)
            os.replace(manifest, receipt)
    return binary


if __name__ == '__main__':
    try:
        binary = prepare()
        if sys.argv[1:] == ['prepare']:
            print(binary)
        else:
            os.execv(str(binary), [str(binary), *sys.argv[1:]])
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f'distribution-tool: {error}', file=sys.stderr)
        sys.exit(1)
