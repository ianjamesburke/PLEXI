import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('test_package', Path(__file__).with_name('test-package.py'))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class MacOSDeploymentTargetTests(unittest.TestCase):
    def test_checks_every_slice_and_rejects_newer_minimum(self):
        output = 'Load command 1\n      cmd LC_BUILD_VERSION\n    minos 14.0\nLoad command 2\n      cmd LC_BUILD_VERSION\n    minos 15.0\n'
        with patch.object(package.subprocess, 'check_output', return_value=output):
            with self.assertRaisesRegex(ValueError, '15.0'):
                package.verify_macos_minimum(Path('binary'), '14.0')

    def test_accepts_older_or_equal_and_rejects_missing_metadata(self):
        with patch.object(package.subprocess, 'check_output', return_value='    minos 12.0\n    minos 14.0\n'):
            package.verify_macos_minimum(Path('binary'), '14.0')
        with patch.object(package.subprocess, 'check_output', return_value='no build version'):
            with self.assertRaisesRegex(ValueError, 'minimum'):
                package.verify_macos_minimum(Path('binary'), '14.0')


if __name__ == '__main__':
    unittest.main()
