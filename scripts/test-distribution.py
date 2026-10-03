#!/usr/bin/env python3
"""Native transaction tests. Host fixtures isolate installation from rendering."""
import argparse
from contextlib import ExitStack
import hashlib
import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
import tarfile
import zipfile

REPO = Path(__file__).resolve().parent.parent


def sha(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def run(command, env, success=True):
    result = subprocess.run(list(map(str, command)), env=env, cwd=env['HOME'], text=True, capture_output=True, timeout=90)
    if (result.returncode == 0) != success:
        raise AssertionError(f'{command}: exit={result.returncode}\n{result.stdout}\n{result.stderr}')
    return result


def package(root, installer, platform, channel, build):
    result = root / f'package-{channel}-{build}'
    result.mkdir()
    name = 'plexi' if channel == 'stable' else 'plexi-' + channel
    resources = result / 'resources'
    if platform.startswith('macos-'):
        display = 'Plexi' if channel == 'stable' else 'Plexi ' + channel.capitalize()
        executable = result / (display + '.app') / 'Contents/MacOS' / name
    else:
        executable = result / (name + ('.exe' if os.name == 'nt' else ''))
    executable.parent.mkdir(parents=True, exist_ok=True)
    source = root / f'{build}.rs'
    identity = dict(version='1.0.0' if build == 'old' else '1.1.0', tag='v1.0.0' if build == 'old' else 'v1.1.0', build_id=build, source_commit=build)
    literal = json.dumps(json.dumps(identity))
    source.write_text('''use std::{env,fs,path::PathBuf,thread,time::Duration};
fn main() {
    if env::args().nth(1).as_deref() == Some("--hold") {
        fs::write(env::var("DISTRIBUTION_TEST_HOLD").unwrap(), "ready").unwrap();
        loop { thread::sleep(Duration::from_millis(100)); }
    }
    if env::args().nth(1).as_deref() == Some("--distribution-check") {
        if let Ok(gate) = env::var("DISTRIBUTION_TEST_GATE") {
            let path = PathBuf::from(&gate);
            let count: usize = fs::read_to_string(&path).unwrap_or_default().parse().unwrap_or(0);
            fs::write(&path,(count+1).to_string()).unwrap();
            if count >= 1 {
                fs::write(path.with_extension("ready"), std::process::id().to_string()).unwrap();
                loop { thread::sleep(Duration::from_millis(100)); }
            }
        }
    }
    println!("{}", ''' + literal + ''');
}
''')
    run(['rustc', source, '-o', executable], os.environ.copy())
    shutil.copy2(installer, result / ('plexi-installer.exe' if os.name == 'nt' else 'plexi-installer'))
    for name in ['wasm-bundles/cpython-3.12.12/python.wasm', 'wasm-bundles/cpython-3.12.12/Lib/encodings/__init__.py', 'sdk/plexi_sdk/_v3_process.py', 'smoke-app/manifest.toml']:
        path = resources / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text('transaction fixture\n')
    manifest = identity | dict(schema=1, channel=channel, platform=platform, executable=executable.relative_to(result).as_posix(), resources='resources')
    manifest['files'] = {p.relative_to(result).as_posix(): sha(p) for p in result.rglob('*') if p.is_file()}
    (result / 'package.json').write_text(json.dumps(manifest))
    return result


def main(args):
    with tempfile.TemporaryDirectory(prefix='plexi distribution tests ') as directory, ExitStack() as cleanup:
        root = Path(directory)
        home = root / 'home with spaces'
        home.mkdir()
        env = {k: v for k, v in os.environ.items() if not k.startswith('PLEXI_')}
        env.pop('ZDOTDIR', None)
        env.update(HOME=str(home), USERPROFILE=str(home), XDG_DATA_HOME=str(home / 'data'), LOCALAPPDATA=str(home / 'local'), APPDATA=str(home / 'roaming'))
        env['PLEXI_DISTRIBUTION_HOME'] = str(home / 'distribution')
        env['PLEXI_INSTALL_DIR'] = str(root / 'custom install')
        env['PLEXI_BIN_DIR'] = str(root / 'command bin')
        apps = home / 'Applications'
        installer = args.installer.resolve()
        def remove_test_installations():
            # Windows Known Folders and user PATH are account-scoped even when
            # HOME/APPDATA are overridden. Remove all receipt-owned integrations
            # before deleting the temporary registry and payloads.
            for receipt in json.loads(run([installer, 'list'], env).stdout):
                run([installer, 'remove', '--receipt', receipt['root']], env)
            assert json.loads(run([installer, 'list'], env).stdout) == []
        cleanup.callback(remove_test_installations)
        old = package(root, installer, args.platform, 'stable', 'old')
        new = package(root, installer, args.platform, 'stable', 'new')
        alpha = package(root, installer, args.platform, 'alpha', 'new')
        def install(path, success=True, custom_env=env):
            channel = json.loads((path / 'package.json').read_text())['channel']
            return run([installer, '--package', path, '--channel', channel, '--applications-dir', apps, '--install-only'], custom_env, success)
        receipt_path = root / 'custom install/stable/installation.json'
        def active(): return json.loads(receipt_path.read_text())['active']['build_id']
        initial = install(old)
        assert active() == 'old'
        if os.name == 'nt':
            path_command = next(line.split(': ', 1)[1] for line in initial.stdout.splitlines() if line.startswith('For this PowerShell session:'))
            output = run(['powershell', '-NoProfile', '-Command', path_command + '; plexi --build-info'], env)
            assert json.loads(output.stdout)['build_id'] == 'old'
            launch_command = next(line.split(': ', 1)[1] for line in initial.stdout.splitlines() if line.startswith('Launch:'))
            assert json.loads(run(['powershell', '-NoProfile', '-Command', launch_command], env).stdout)['build_id'] == 'old'
        if os.name != 'nt':
            # Ordinary interactive shells must discover the owned command even
            # when an older unrelated command precedes it in the inherited PATH.
            stale = root / 'stale system bin'
            stale.mkdir()
            shadow = stale / 'plexi'
            shadow.write_text('#!/bin/sh\necho stale\n')
            shadow.chmod(0o755)
            shell_env = env | {'PATH': str(stale) + os.pathsep + env['PATH']}
            output = run(['bash', '--noprofile', '--rcfile', home / '.bashrc', '-ic', 'plexi --build-info'], shell_env)
            assert json.loads(output.stdout)['build_id'] == 'old'
            assert shadow.read_text().endswith('echo stale\n')
        install(alpha)
        listed = json.loads(run([installer, 'list'], env).stdout)
        assert {r['channel'] for r in listed} == {'stable', 'alpha'}
        assert Path(run([installer, 'locate', '--channel', 'stable'], env).stdout.strip()).is_file()
        assert run([installer, 'locate', '--channel', 'pr-999'], env, False).returncode == 2
        retained = home / '.plexi/user-document.txt'
        retained.parent.mkdir(exist_ok=True)
        retained.write_text('keep me')
        generation = json.loads(receipt_path.read_text())['active']
        held = subprocess.Popen([str(Path(generation['path']) / generation['executable']), '--hold'], env=env | {'DISTRIBUTION_TEST_HOLD': str(root / 'held')}, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            deadline = time.monotonic() + 10
            while not (root / 'held').exists():
                if time.monotonic() > deadline: raise TimeoutError('running executable fixture')
                time.sleep(.05)
            install(new)
            assert held.poll() is None, 'upgrade replaced or terminated the running generation'
        finally:
            held.kill(); held.wait(timeout=10)
        assert active() == 'new'
        assert json.loads(receipt_path.read_text())['previous']['build_id'] == 'old'
        run([installer, 'rollback', '--receipt', receipt_path.parent], env)
        assert active() == 'old'
        print('PASS: channel coexistence, spaces, upgrade and rollback')
        payload = new / 'resources/sdk/plexi_sdk/_v3_process.py'
        original = payload.read_bytes()
        payload.write_bytes(b'corrupted')
        install(new, False)
        assert active() == 'old'
        payload.write_bytes(original)
        print('PASS: corrupted payload preserves active generation')
        gate = root / 'check-gate'
        interrupted = env | {'DISTRIBUTION_TEST_GATE': str(gate)}
        with (root / 'interrupt.log').open('w') as log:
            proc = subprocess.Popen([str(installer), '--package', str(new), '--install-only'], env=interrupted, cwd=home, stdout=log, stderr=log)
            try:
                deadline = time.monotonic() + 30
                while not gate.with_suffix('.ready').exists():
                    if proc.poll() is not None: raise AssertionError((root / 'interrupt.log').read_text())
                    if time.monotonic() > deadline: raise TimeoutError('activation gate')
                    time.sleep(.05)
                install(new, False)
                proc.kill()
                proc.wait(timeout=10)
            finally:
                if proc.poll() is None: proc.kill(); proc.wait()
        # The transaction's worker is a fixture process; release it too.
        os.kill(int(gate.with_suffix('.ready').read_text()), 9)
        launcher = root / ('command bin/plexi.exe' if os.name == 'nt' else 'command bin/plexi')
        result = run([launcher, '--build-info'], env)
        assert json.loads(result.stdout)['build_id'] == 'old'
        assert active() == 'old'
        print('PASS: simultaneous installer refusal and SIGKILL recovery through launcher')
        # Retained data and an unrelated command survive channel removal.
        outsider = root / 'command bin/plexi-development'
        outsider.write_text('user-owned')
        run([installer, 'remove', '--receipt', receipt_path.parent], env)
        assert not receipt_path.exists()
        assert retained.read_text() == 'keep me'
        assert outsider.read_text() == 'user-owned'
        assert (root / 'custom install/alpha/installation.json').is_file()
        install(old)
        assert active() == 'old'
        print('PASS: scoped uninstall and reinstall retain user data and other channels')
        # Exercise real HTTP, archive and checksum handling from the public bootstrap.
        server_root = root / 'server'
        tag_dir = server_root / 'download/v1.1.0'
        tag_dir.mkdir(parents=True)
        asset = f'plexi-{args.platform}.' + ('zip' if os.name == 'nt' else 'tar.gz')
        archive = tag_dir / asset
        if os.name == 'nt':
            with zipfile.ZipFile(archive, 'w') as z:
                for path in new.rglob('*'):
                    if path.is_file(): z.write(path, path.relative_to(new))
        else:
            with tarfile.open(archive, 'w:gz') as tar:
                for path in new.iterdir(): tar.add(path, path.name)
        sidecar = tag_dir / (asset + '.sha256')
        sidecar.write_text(sha(archive) + '  ' + asset + '\n')
        helper = f'plexi-installer-{args.platform}' + ('.exe' if os.name == 'nt' else '')
        shutil.copy2(installer, tag_dir / helper)
        (tag_dir / (helper + '.sha256')).write_text(sha(tag_dir / helper) + '  ' + helper + '\n')
        release = dict(tag_name='v1.1.0', draft=False, prerelease=False, assets=[dict(name=asset), dict(name=asset+'.sha256')])
        class Handler(http.server.SimpleHTTPRequestHandler):
            def __init__(self, *a, **kw): super().__init__(*a, directory=str(server_root), **kw)
            def log_message(self, *a): pass
            def do_GET(self):
                if self.path.startswith('/releases'):
                    body = json.dumps(release if self.path.startswith('/releases/latest') else [release]).encode()
                    self.send_response(200); self.end_headers(); self.wfile.write(body)
                else: super().do_GET()
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
        base = f'http://127.0.0.1:{server.server_port}'
        http_env = env | {'PLEXI_RELEASES_URL': base + '/releases', 'PLEXI_RELEASE_BASE_URL': base + '/download'}
        try:
            bootstrap = ['powershell', '-NoProfile', '-File', REPO / 'scripts/install-windows.ps1', '-InstallOnly'] if os.name == 'nt' else ['bash', REPO / 'install.sh', '--install-only']
            run(bootstrap, http_env)
            assert active() == 'new'
            sidecar.write_text('0'*64 + '  ' + asset + '\n')
            run([installer, '--install-only'], http_env, False)
            sidecar.unlink()
            run([installer, '--install-only'], http_env, False)
            run(bootstrap, http_env | {'PLEXI_RELEASE_BASE_URL': base + '/missing'}, False)
            assert active() == 'new'
            print('PASS: exact bootstrap, missing assets, bad checksums and failed downloads')
        finally:
            server.shutdown(); server.server_close(); thread.join()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--installer', type=Path, required=True)
    parser.add_argument('--platform', required=True)
    main(parser.parse_args())
