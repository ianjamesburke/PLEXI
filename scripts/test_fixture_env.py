#!/usr/bin/env python3
"""Cover the fixture locale helper: discovery cases, inherited settings, real Perl and UTF-8 round trips."""
import shutil
import subprocess
import unittest
from unittest import mock

import fixture_env
from fixture_env import utf8_env

LINUX = {"c", "posix", "c.utf8", "en_us.utf8"}
MACOS = {"c", "posix", "en_us.utf8"}
BOGUS = "xx_XX.UTF-8"


class Selection(unittest.TestCase):
    def test_prefers_c_utf8_when_available(self):
        env = utf8_env({"LANG": BOGUS, "PATH": "/bin"}, LINUX)
        self.assertEqual((env["LC_ALL"], env["LANG"], env["PATH"]), ("C.UTF-8", "C.UTF-8", "/bin"))

    def test_macos_falls_back_to_en_us(self):
        env = utf8_env({"LC_ALL": "C.UTF-8", "LANG": "C.UTF-8", "LC_CTYPE": "C.UTF-8"}, MACOS)
        self.assertEqual((env["LC_ALL"], env["LANG"]), ("en_US.UTF-8", "en_US.UTF-8"))
        self.assertNotIn("LC_CTYPE", env)

    def test_valid_inherited_locale_is_preserved(self):
        base = {"LANG": "en_US.UTF-8", "LC_CTYPE": "en_US.UTF-8", "HOME": "/h"}
        self.assertEqual(utf8_env(base, LINUX), base)

    def test_valid_inherited_non_utf8_is_replaced(self):
        env = utf8_env({"LANG": "C"}, LINUX)
        self.assertEqual(env["LC_ALL"], "C.UTF-8")

    def test_one_unsupported_variable_triggers_replacement(self):
        env = utf8_env({"LANG": "en_US.UTF-8", "LC_MESSAGES": BOGUS}, LINUX)
        self.assertNotIn("LC_MESSAGES", env)
        self.assertEqual(env["LC_ALL"], "C.UTF-8")

    def test_unset_variables_get_a_supported_locale(self):
        self.assertEqual(utf8_env({"PATH": "/bin"}, LINUX)["LC_ALL"], "C.UTF-8")

    def test_empty_values_count_as_unset(self):
        env = utf8_env({"LC_ALL": "", "LANG": "en_US.UTF-8"}, LINUX)
        self.assertEqual(env, {"LC_ALL": "", "LANG": "en_US.UTF-8"})

    def test_no_installed_utf8_locale_drops_every_locale_variable(self):
        env = utf8_env({"LANG": BOGUS, "LC_ALL": BOGUS, "PATH": "/bin"}, {"c", "posix"})
        self.assertEqual(env, {"PATH": "/bin"})

    def test_unavailable_discovery_drops_every_locale_variable(self):
        with mock.patch.object(fixture_env.subprocess, "run", side_effect=FileNotFoundError("locale")):
            self.assertIsNone(fixture_env.discover_locales())
            self.assertEqual(utf8_env({"LANG": BOGUS, "PATH": "/bin"}), {"PATH": "/bin"})

    def test_input_is_not_mutated(self):
        base = {"LANG": BOGUS}
        utf8_env(base, LINUX)
        self.assertEqual(base, {"LANG": BOGUS})

    def test_normalizes_linux_and_macos_spellings(self):
        self.assertEqual(fixture_env._normalize("en_US.UTF-8"), fixture_env._normalize("en_US.utf8"))


@unittest.skipUnless(shutil.which("perl"), "perl not installed")
class RealProcesses(unittest.TestCase):
    def perl(self, env):
        return subprocess.run(["perl", "-e", "print qq(ok\\n)"], env=env, capture_output=True, text=True)

    def test_negative_control_unsupported_locale_warns(self):
        result = self.perl({"PATH": "/usr/bin:/bin", "LC_ALL": BOGUS, "LANG": BOGUS})
        self.assertIn("locale", result.stderr.lower())

    def test_controlled_env_is_clean(self):
        env = utf8_env({"PATH": "/usr/bin:/bin", "LC_ALL": BOGUS, "LANG": BOGUS})
        result = self.perl(env)
        self.assertEqual((result.stdout, result.stderr), ("ok\n", ""))

    def test_unicode_round_trip(self):
        env = utf8_env({"PATH": "/usr/bin:/bin", "LC_ALL": BOGUS, "LANG": BOGUS})
        result = subprocess.run(["python3", "-c", "import sys; sys.stdout.write(sys.stdin.read())"],
                                input="café ✓".encode("utf-8"), env=env, capture_output=True)
        self.assertEqual(result.stdout.decode("utf-8"), "café ✓")


if __name__ == "__main__":
    unittest.main()
