import hashlib
import os
import re
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('publish-release.sh')


class PublishReleaseTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.dist = self.root / 'dist'
        self.dist.mkdir()
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        gh = self.bin / 'gh'
        gh.write_text('#!/bin/bash\nif [[ "$1 $2" == "release view" ]]; then exit 1; fi\nprintf "%s\\n" "$*" >> "$GH_CALL_LOG"\n')
        gh.chmod(0o755)
        self.env = dict(os.environ, PATH=f'{self.bin}:{os.environ["PATH"]}', GH_CALL_LOG=str(self.root / 'gh.log'))

    def tearDown(self):
        self.tmp.cleanup()

    def assets(self, platform, channels):
        ext = 'zip' if platform.startswith('windows-') else 'tar.gz'
        exe = '.exe' if platform.startswith('windows-') else ''
        names = [f'plexi-{platform}{suffix}.{ext}' for suffix in channels] + [f'plexi-installer-{platform}{exe}']
        for name in names:
            data = name.encode()
            (self.dist / name).write_bytes(data)
            (self.dist / (name + '.sha256')).write_text(f'{hashlib.sha256(data).hexdigest()}  {name}\n')

    def run_publish(self, tag='v0.4.4'):
        return subprocess.run(['bash', str(SCRIPT), tag, str(self.dist)], env=self.env, text=True, capture_output=True)

    def test_stable_mac_only_succeeds_and_logs_absent_optional_platforms(self):
        self.assets('macos-arm64', ['', '-alpha', '-beta'])
        result = self.run_publish()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for platform in ['linux-x64', 'windows-x64', 'macos-x64']:
            self.assertIn(platform, result.stdout + result.stderr)
        self.assertIn('release upload', (self.root / 'gh.log').read_text())

    def test_optional_incomplete_or_bad_checksum_is_omitted(self):
        self.assets('macos-arm64', ['', '-alpha', '-beta'])
        self.assets('linux-x64', ['', '-alpha', '-beta'])
        (self.dist / 'plexi-linux-x64.tar.gz').write_text('corrupt')
        (self.dist / 'plexi-windows-x64.zip').write_text('partial')
        result = self.run_publish()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('linux-x64', result.stdout + result.stderr)
        self.assertIn('windows-x64', result.stdout + result.stderr)
        self.assertNotIn('plexi-linux-x64', (self.root / 'gh.log').read_text())

    def test_required_mac_missing_fails_before_gh(self):
        result = self.run_publish()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('macos-arm64', result.stderr)
        self.assertFalse((self.root / 'gh.log').exists())

    def test_signing_workflow_uses_packaged_channel_archive_names(self):
        workflow = SCRIPT.parent.parent / '.github/workflows/distribution.yml'
        match = re.search(r'--archive "(dist/plexi-.*?)" --expected-authority', workflow.read_text())
        self.assertIsNotNone(match)
        expression = match.group(1).replace('${{ matrix.platform }}', 'macos-arm64')
        lines = re.findall(r'^\s+(archive_suffix=.*)$', workflow.read_text(), re.M)
        self.assertEqual(len(lines), 2)
        for channel, suffix in [('stable', ''), ('beta', '-beta'), ('alpha', '-alpha')]:
            script = 'channel=$1; ' + '; '.join(lines) + '; printf "%s" "' + expression + '"'
            actual = subprocess.check_output(['bash', '-c', script, '_', channel], text=True)
            self.assertEqual(actual, f'dist/plexi-macos-arm64{suffix}.tar.gz')

    def test_alpha_requires_only_alpha_mac_assets(self):
        self.assets('macos-arm64', ['-alpha'])
        result = self.run_publish('v0.4.4-alpha.1')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('linux-x64', result.stdout + result.stderr)
        self.assertIn('plexi-macos-arm64-alpha.tar.gz', (self.root / 'gh.log').read_text())

    def test_beta_also_accepts_mac_only(self):
        self.assets('macos-arm64', ['', '-alpha', '-beta'])
        result = self.run_publish('v0.4.4-beta.1')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('windows-x64', result.stdout + result.stderr)

    def test_complete_optional_platform_is_published(self):
        self.assets('macos-arm64', ['', '-alpha', '-beta'])
        self.assets('windows-x64', ['', '-alpha', '-beta'])
        result = self.run_publish()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('plexi-windows-x64.zip', (self.root / 'gh.log').read_text())


if __name__ == '__main__':
    unittest.main()
