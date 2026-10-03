#!/usr/bin/env python3
"""Verify a real package in a fresh profile, optionally driving its native GUI."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time


def main(args):
    package = args.package.resolve()
    manifest = json.loads((package / 'package.json').read_text())
    args.output.mkdir(parents=True, exist_ok=True)
    # Keep Unix socket paths below sockaddr_un.sun_path on macOS.
    with tempfile.TemporaryDirectory(prefix='plexi pkg ', dir='/tmp' if os.name != 'nt' else None) as temporary:
        root = Path(temporary)
        home = root / 'home'
        home.mkdir()
        env = {k: v for k, v in os.environ.items() if not k.startswith('PLEXI_')}
        env.pop('ZDOTDIR', None)
        env.update(HOME=str(home), USERPROFILE=str(home), XDG_DATA_HOME=str(home / 'data'), LOCALAPPDATA=str(home / 'local'), APPDATA=str(home / 'roaming'), PLEXI_DISTRIBUTION_HOME=str(home / 'distribution'))
        def run(command, timeout=120):
            result = subprocess.run(list(map(str, command)), cwd=home, env=env, capture_output=True, text=True, timeout=timeout)
            with (args.output / 'commands.log').open('a') as log:
                log.write(f'{command}\nexit={result.returncode}\n{result.stdout}\n{result.stderr}\n')
            if result.returncode:
                raise RuntimeError(f'{command}: {result.stderr[-4000:]}')
            return result.stdout
        installer = package / ('plexi-installer.exe' if os.name == 'nt' else 'plexi-installer')
        run([installer, '--package', package, '--channel', manifest['channel'], '--install-dir', root / 'install', '--bin-dir', root / 'bin', '--applications-dir', home / 'Applications', '--install-only'])
        receipt_path = root / 'install' / manifest['channel'] / 'installation.json'
        receipt = json.loads(receipt_path.read_text())
        executable = Path(receipt['active']['path']) / receipt['active']['executable']
        identity = json.loads(run([executable, '--distribution-check']))
        assert identity['build_id'] == manifest['build_id']
        if args.gui:
            started = False
            try:
                started = True
                run([executable, 'host', 'start', '--ephemeral', '--pane', f'cwd={home}', '--timeout-secs', '90'])
                status = json.loads(run([executable, 'host', 'status', '--json']))
                assert status['ready'] and status['running']['build_id'] == manifest['build_id'], status
                env['PLEXI_SOCKET'] = status['socket']
                if manifest['channel'] != 'stable': env['PLEXI_CHANNEL'] = manifest['channel']
                panes = json.loads(run([executable, 'pane', 'list']))
                terminal = panes[0]['id']
                run([executable, 'pane', 'name', str(terminal), 'Distribution terminal'])
                run([executable, 'pane', 'send', str(terminal), 'echo PLEXI_DISTRIBUTION_OK', '--submit'])
                deadline = time.monotonic() + 15
                while True:
                    capture = run([executable, 'pane', 'capture', str(terminal), '--plain'])
                    if re.search(r'^\s*PLEXI_DISTRIBUTION_OK\s*$', capture, re.M): break
                    if time.monotonic() > deadline: raise AssertionError('terminal never produced the expected output')
                    time.sleep(.2)
                run([executable, 'app', 'open', 'calc'])
                deadline = time.monotonic() + 60
                while True:
                    panes = json.loads(run([executable, 'pane', 'list']))
                    candidates = [p for p in panes if p['type'] == 'app']
                    if candidates:
                        app = candidates[-1]['id']
                        state = run([executable, 'pane', 'state', str(app)])
                        if 'Calculator' in state or 'button' in state.lower(): break
                    if time.monotonic() > deadline: raise AssertionError('Calculator never produced a semantic frame')
                    time.sleep(.2)
                run([executable, 'pane', 'name', str(app), 'Packaged Calculator'])
                run([executable, 'host', 'screenshot', '--output', args.output / 'installed-host.png'])
                # Restart uses a retained package even while the original process exists.
                run([executable, 'host', 'stop'])
                started = False
                env.pop('PLEXI_SOCKET', None)
                run([executable, 'host', 'start', '--ephemeral', '--timeout-secs', '90'])
                started = True
                assert json.loads(run([executable, 'host', 'status', '--json']))['ready']
            finally:
                profile = home / ('.plexi' if manifest['channel'] == 'stable' else '.plexi-' + manifest['channel'])
                for log in profile.glob('*.log'):
                    shutil.copy2(log, args.output / log.name)
                if started:
                    run([executable, 'host', 'stop'])
                    assert not json.loads(run([executable, 'host', 'status', '--json']))['ready']
        # Removal runs from the unpacked installer, so it can also delete the
        # Windows application executable and stable launcher without a file lock.
        env.pop('PLEXI_SOCKET', None)
        run([installer, 'remove', '--receipt', receipt_path.parent])
        assert not receipt_path.exists()
        (args.output / 'result.json').write_text(json.dumps({'channel': manifest['channel'], 'build_id': manifest['build_id'], 'runtime': 'pass', 'gui': 'pass' if args.gui else 'not-run', 'removal': 'pass'}, indent=2))


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--package', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--gui', action='store_true')
    main(p.parse_args())
