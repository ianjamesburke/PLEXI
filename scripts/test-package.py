#!/usr/bin/env python3
"""Verify a real package in a fresh profile, optionally driving its native GUI."""
import argparse
import json
import hashlib
import http.server
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time
import threading

REPO = Path(__file__).resolve().parent.parent


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
        env.update(HOME=str(home), USERPROFILE=str(home), XDG_DATA_HOME=str(home / 'data'), XDG_CONFIG_HOME=str(home / '.config'), LOCALAPPDATA=str(home / 'local'), APPDATA=str(home / 'roaming'), PLEXI_DISTRIBUTION_HOME=str(home / 'distribution'))
        def run(command, timeout=120):
            result = subprocess.run(list(map(str, command)), cwd=home, env=env, capture_output=True, text=True, timeout=timeout)
            with (args.output / 'commands.log').open('a') as log:
                log.write(f'{command}\nexit={result.returncode}\n{result.stdout}\n{result.stderr}\n')
            if result.returncode:
                raise RuntimeError(f'{command}: {result.stderr[-4000:]}')
            return result.stdout
        # Windows Known Folders do not follow HOME/USERPROFILE overrides; native
        # Windows runs use the disposable runner account's real profile.
        profile_home = Path(run(['powershell', '-NoProfile', '-Command', "[Environment]::GetFolderPath('UserProfile')"]).strip()) if os.name == 'nt' else home
        installer = package / ('plexi-installer.exe' if os.name == 'nt' else 'plexi-installer')
        auto_started = False
        tag, channel = manifest['tag'], manifest['channel']
        accepted = channel == 'alpha' or (channel == 'beta' and '-alpha.' not in tag) or (channel == 'stable' and '-' not in tag)
        if accepted:
            # Serve the actual release archive, then execute the public bootstrap.
            # Close the server before GUI launch to prove resources are local.
            suffix = '' if channel == 'stable' else '-' + channel
            archive_name = f"plexi-{manifest['platform']}{suffix}." + ('zip' if os.name == 'nt' else 'tar.gz')
            server_root = root / 'server'
            assets = server_root / 'download' / tag
            assets.mkdir(parents=True)
            for name in [archive_name, archive_name + '.sha256']:
                shutil.copy2(package.parent / name, assets / name)
            helper = f"plexi-installer-{manifest['platform']}" + ('.exe' if os.name == 'nt' else '')
            shutil.copy2(installer, assets / helper)
            with installer.open('rb') as source:
                checksum = hashlib.file_digest(source, 'sha256').hexdigest()
            (assets / (helper + '.sha256')).write_text(checksum + '  ' + helper + '\n')
            release = dict(tag_name=tag, draft=False, prerelease='-' in tag, assets=[dict(name=archive_name), dict(name=archive_name + '.sha256')])
            class Handler(http.server.SimpleHTTPRequestHandler):
                def __init__(self, *a, **kw): super().__init__(*a, directory=str(server_root), **kw)
                def log_message(self, *a): pass
                def do_GET(self):
                    if self.path.startswith('/releases'):
                        self.send_response(200); self.end_headers(); self.wfile.write(json.dumps([release]).encode())
                    else: super().do_GET()
            server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
            worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
            env.update(PLEXI_BOOTSTRAP_TAG=tag, PLEXI_RELEASES_URL=f'http://127.0.0.1:{server.server_port}/releases', PLEXI_RELEASE_BASE_URL=f'http://127.0.0.1:{server.server_port}/download', PLEXI_INSTALL_DIR=str(root / 'install'), PLEXI_BIN_DIR=str(root / 'bin'))
            try:
                bootstrap = ['powershell', '-NoProfile', '-File', REPO / 'scripts/install-windows.ps1', '-Channel', channel] if os.name == 'nt' else ['bash', REPO / 'install.sh', '--channel', channel]
                if not args.gui: bootstrap.append('-InstallOnly' if os.name == 'nt' else '--install-only')
                run(bootstrap, timeout=300)
                auto_started = args.gui
            finally:
                server.shutdown(); server.server_close(); worker.join()
                for key in ['PLEXI_BOOTSTRAP_TAG', 'PLEXI_RELEASES_URL', 'PLEXI_RELEASE_BASE_URL']:
                    env.pop(key)
        else:
            run([installer, '--package', package, '--channel', channel, '--install-dir', root / 'install', '--bin-dir', root / 'bin', '--applications-dir', home / 'Applications', '--install-only'])
        receipt_path = root / 'install' / manifest['channel'] / 'installation.json'
        receipt = json.loads(receipt_path.read_text())
        executable = Path(receipt['active']['path']) / receipt['active']['executable']
        check = [executable, '--distribution-check']
        if sys.platform == 'darwin':
            profile = '(version 1) (allow default) (deny network*) (deny file-read* (subpath ' + json.dumps(str(REPO)) + '))'
            check = ['/usr/bin/sandbox-exec', '-p', profile] + check
        identity = json.loads(run(check))
        assert identity['build_id'] == manifest['build_id']
        if os.name != 'nt' and not channel.startswith('pr-'):
            name = 'plexi' if channel == 'stable' else 'plexi-' + channel
            completion = receipt_path.parent / 'completions/zsh' / ('_' + name)
            assert completion.is_file(), 'package install omitted shell completions'
            if shutil.which('zsh'):
                # A fresh interactive shell must define the actual generated
                # function, not merely cache its missing autoload name.
                run(['zsh', '-ic', f'[[ ${{_comps[{name}]}} == _{name} ]] && [[ $functions[_{name}] != *"autoload -X"* ]]'])
                run(['zsh', '-n', completion])
            run(['bash', '--noprofile', '--rcfile', home / '.bashrc', '-ic', f'complete -p {name}'])
        if args.gui:
            started = False
            try:
                started = True
                if not auto_started:
                    run([executable, 'host', 'start', '--ephemeral', '--pane', f'cwd={home}', '--timeout-secs', '90'])
                status = json.loads(run([executable, 'host', 'status', '--json']))
                assert status['ready'] and status['running']['build_id'] == manifest['build_id'], status
                if os.name != 'nt':
                    hook = profile_home / '.plexi/hooks/claude-code-agent-state.sh'
                    assert hook.is_file() and os.access(hook, os.X_OK), 'host startup did not restore shared hook'
                    run([hook])  # Outside a pane the registered hook exits cleanly.
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
                # Exercise the same helper used by the restart badge: schedule
                # before shutdown, wait for that exact process, then launch the receipt.
                with (args.output / 'restart.log').open('w') as restart_log:
                    helper = subprocess.Popen([str(installer), 'restart', '--receipt', str(receipt_path.parent), '--wait-pid', str(status['running']['pid'])], cwd=home, env=env, stdout=restart_log, stderr=restart_log)
                    try:
                        run([executable, 'host', 'stop'])
                        env.pop('PLEXI_SOCKET', None)
                        assert helper.wait(timeout=120) == 0, 'restart helper failed'
                    finally:
                        if helper.poll() is None: helper.kill(); helper.wait()
                restarted = json.loads(run([executable, 'host', 'status', '--json']))
                assert restarted['ready'] and restarted['running']['pid'] != status['running']['pid']
                assert restarted['running']['build_id'] == manifest['build_id']
            finally:
                profile = profile_home / ('.plexi' if manifest['channel'] == 'stable' else '.plexi-' + manifest['channel'])
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
