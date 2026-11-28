import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


scripts = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('test_package', scripts / 'test-package.py')
package = importlib.util.module_from_spec(spec)
with patch.object(sys, 'path', [str(scripts), *sys.path]):
    spec.loader.exec_module(package)


class PackageProfileTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.account = Path(temp.name) / 'account'
        self.account.mkdir()
        self.home = Path(temp.name) / 'home'
        self.home.mkdir()

    def test_pr_uses_temp_home(self):
        self.assertEqual(package.profile_home_for('pr-3320', self.home, self.account), self.home)

    def test_release_uses_account_home(self):
        for channel in ('alpha', 'beta', 'stable'):
            with self.subTest(channel=channel):
                self.assertEqual(package.profile_home_for(channel, self.home, self.account), self.account)

    def test_requires_disposable_account(self):
        with self.assertRaisesRegex(SystemExit, 'disposable account'):
            package.check_disposable(self.account, {'GITHUB_ACTIONS': 'false'})

    def gui_args(self, channel):
        manifest = {'tag': 'v1.0.0', 'channel': channel, 'platform': 'linux-x64',
                    'executable': f'plexi-{channel}'}
        (self.home / 'package.json').write_text(json.dumps(manifest))
        return SimpleNamespace(package=self.home, output=self.home / 'logs', gui=True)

    @unittest.skipIf(os.name == 'nt', 'Unix passwd home')
    def test_gui_refuses_before_bootstrap(self):
        args = self.gui_args('alpha')
        record = SimpleNamespace(pw_dir=str(self.account))
        with (patch('pwd.getpwuid', return_value=record),
              patch.dict(os.environ, {}, clear=True),
              patch.object(package.subprocess, 'run') as run):
            with self.assertRaisesRegex(SystemExit, 'disposable account'):
                package.main(args)
            run.assert_not_called()

    @unittest.skipIf(os.name == 'nt', 'Unix PR builds honor HOME')
    def test_pr_label_on_a_release_build_cannot_bypass_guard(self):
        args = self.gui_args('pr-3320')
        record = SimpleNamespace(pw_dir=str(self.account))
        build_info = SimpleNamespace(returncode=0, stdout='{"test_channel":null}', stderr='')
        with (patch('pwd.getpwuid', return_value=record),
              patch.object(package.subprocess, 'run', return_value=build_info) as run):
            with self.assertRaisesRegex(SystemExit, 'must be built for pr-3320'):
                package.main(args)
            self.assertEqual([call.args[0] for call in run.call_args_list],
                             [[str((self.home / 'plexi-pr-3320').resolve()), '--build-info']])

    def test_refuses_a_listening_host_and_a_held_lock(self):
        import fcntl, socket
        profile = self.account / '.plexi-alpha'
        profile.mkdir()
        with socket.socket(socket.AF_UNIX) as server:
            server.bind(str(profile / 'notify.sock'))
            server.listen()
            with self.assertRaisesRegex(SystemExit, 'live host'):
                package.check_disposable(self.account, {'GITHUB_ACTIONS': 'true'})
        (profile / 'notify.sock').unlink()
        with (profile / 'host.lock').open('wb') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            with self.assertRaisesRegex(SystemExit, 'live host'):
                package.check_disposable(self.account, {'GITHUB_ACTIONS': 'true'})

    def test_stale_lock_file_and_socket_from_a_stopped_host_pass(self):
        import socket
        profile = self.account / '.plexi'
        profile.mkdir()
        (profile / 'host.lock').touch()
        with socket.socket(socket.AF_UNIX) as server:
            server.bind(str(profile / 'notify.sock'))
        package.check_disposable(self.account, {'GITHUB_ACTIONS': 'true'})

    def test_clean_disposable_account_passes(self):
        for env in ({'GITHUB_ACTIONS': 'true'}, {'PLEXI_PACKAGE_TEST_DISPOSABLE_ACCOUNT': '1'}):
            with self.subTest(env=env):
                package.check_disposable(self.account, env)


class PersonTerminalTests(unittest.TestCase):
    queued = {'id': 2, 'type': 'terminal', 'spawned_by': 'queue'}
    human = {'id': 1, 'type': 'terminal', 'spawned_by': 'human'}

    def test_picks_human_terminal_in_either_order(self):
        for panes in ([self.queued, self.human], [self.human, self.queued]):
            with self.subTest(order=[p['id'] for p in panes]):
                self.assertEqual(package.person_terminal(panes), 1)

    def test_single_pane_bootstrap(self):
        self.assertEqual(package.person_terminal([self.human]), 1)

    def test_no_human_terminal_names_every_pane(self):
        with self.assertRaisesRegex(AssertionError, r"2.*terminal.*queue"):
            package.person_terminal([self.queued])


if __name__ == '__main__':
    unittest.main()
