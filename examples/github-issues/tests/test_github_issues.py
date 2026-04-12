"""Tests for the github-issues Plexi app."""
import json
import os
import stat
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from plexi_test import AppTestHarness, test_app_lifecycle  # noqa: E402

APP_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "github_issues.py")


# ---------------------------------------------------------------------------
# Helpers — fake `gh` binary used to mock the CLI for tests
# ---------------------------------------------------------------------------

FAKE_ISSUES = [
    {
        "number": 42,
        "title": "Add bezier draw command",
        "state": "OPEN",
        "labels": [
            {"name": "enhancement", "color": "a2eeef"},
            {"name": "P2", "color": "fbca04"},
        ],
        "author": {"login": "alice"},
    },
    {
        "number": 41,
        "title": "Crash on resize",
        "state": "OPEN",
        "labels": [{"name": "bug", "color": "d73a4a"}, {"name": "P1", "color": "b60205"}],
        "author": {"login": "bob"},
    },
    {
        "number": 40,
        "title": "Markdown rendering polish",
        "state": "OPEN",
        "labels": [{"name": "idea", "color": "cccccc"}],
        "author": {"login": "carol"},
    },
]

FAKE_DETAIL = {
    "body": "## Why\n\nThis is the issue body for testing.\n\n- bullet one\n- bullet two\n",
    "comments": [
        {
            "author": {"login": "alice"},
            "body": "First comment.",
            "createdAt": "2026-04-10T12:00:00Z",
        },
        {
            "author": {"login": "bob"},
            "body": "Second comment with more text.",
            "createdAt": "2026-04-11T08:30:00Z",
        },
    ],
}


def make_fake_gh(tmpdir: str, mode: str = "ok") -> str:
    """
    Create a fake `gh` shell script that mimics the gh CLI subset we use.

    mode:
      "ok"           — auth ok, returns FAKE_ISSUES / FAKE_DETAIL
      "not_authed"   — `auth status` exits 1
    """
    issues_json = json.dumps(FAKE_ISSUES)
    detail_json = json.dumps(FAKE_DETAIL)
    auth_exit = "1" if mode == "not_authed" else "0"

    script = f"""#!/usr/bin/env bash
# Fake gh CLI for plexi github-issues tests.
case "$1" in
  auth)
    if [ "$2" = "status" ]; then
      exit {auth_exit}
    fi
    ;;
  issue)
    if [ "$2" = "list" ]; then
      cat <<'JSON'
{issues_json}
JSON
      exit 0
    fi
    if [ "$2" = "view" ]; then
      cat <<'JSON'
{detail_json}
JSON
      exit 0
    fi
    ;;
esac
echo "fake gh: unhandled args: $@" >&2
exit 1
"""
    path = os.path.join(tmpdir, "gh")
    with open(path, "w") as f:
        f.write(script)
    os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)
    return path


def make_fake_git_repo(tmpdir: str) -> str:
    """Create a tiny directory with a fake `git` shim that reports a github remote."""
    git_path = os.path.join(tmpdir, "git")
    script = """#!/usr/bin/env bash
if [ "$1" = "remote" ] && [ "$2" = "get-url" ] && [ "$3" = "origin" ]; then
  echo "git@github.com:plexitest/example.git"
  exit 0
fi
exit 1
"""
    with open(git_path, "w") as f:
        f.write(script)
    os.chmod(git_path, os.stat(git_path).st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)
    return git_path


def make_env(tmpdir: str, gh_path: str) -> dict:
    """Build an env that points the app at our fake gh + fake git."""
    env = os.environ.copy()
    env["PLEXI_GH_BIN"] = gh_path
    # Prepend tmpdir to PATH so the fake `git` shim is found before system git.
    env["PATH"] = tmpdir + os.pathsep + env.get("PATH", "")
    env["PYTHONUNBUFFERED"] = "1"
    return env


class HarnessWithEnv(AppTestHarness):
    """AppTestHarness variant that allows passing a custom env."""

    def __init__(self, entry_point: str, launch_dir: str, env: dict):
        # Mirror the parent constructor but with a custom env.
        import subprocess as sp
        self.entry_point = os.path.abspath(entry_point)
        self.launch_dir = launch_dir
        self._last_frames = []
        import queue as q
        self._output_queue = q.Queue()
        self._stderr_lines = []
        self._reader_thread = None
        self._stderr_thread = None
        self._closed = False

        os.makedirs(launch_dir, exist_ok=True)
        try:
            self._proc = sp.Popen(
                [sys.executable, self.entry_point],
                stdin=sp.PIPE,
                stdout=sp.PIPE,
                stderr=sp.PIPE,
                cwd=launch_dir,
                env=env,
            )
        except Exception as e:
            raise RuntimeError(f"Failed to spawn app {self.entry_point}: {e}") from e

        import threading as th
        self._reader_thread = th.Thread(target=self._read_stdout, daemon=True)
        self._reader_thread.start()
        self._stderr_thread = th.Thread(target=self._read_stderr, daemon=True)
        self._stderr_thread.start()


def render_until(harness, predicate, max_renders: int = 20, sleep_s: float = 0.05):
    """Repeatedly send render events until predicate(frames) is true or limit reached."""
    last = []
    for _ in range(max_renders):
        frames = harness.send_render()
        last = frames
        if predicate(frames):
            return frames
        time.sleep(sleep_s)
    return last


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

def test_lifecycle():
    """App starts, renders, and shuts down without crashing."""
    # Use the system gh — this just smoke-tests init/render/shutdown.
    test_app_lifecycle(APP_PATH)


def test_preflight_error_state_renders():
    """If gh auth fails, the error screen shows the fix command."""
    with tempfile.TemporaryDirectory() as tmpdir:
        gh = make_fake_gh(tmpdir, mode="not_authed")
        make_fake_git_repo(tmpdir)
        env = make_env(tmpdir, gh)

        with HarnessWithEnv(APP_PATH, tmpdir, env) as h:
            h.send_init()
            frames = render_until(
                h,
                lambda fr: any("not authenticated" in t.lower() or "gh auth login" in t
                               for t in h.find_texts(fr)),
            )
            texts = h.find_texts(frames)
            joined = " | ".join(texts)
            assert "gh auth login" in joined, \
                f"Expected fix command 'gh auth login' in error screen.\nGot: {joined}"
    print("PASS: test_preflight_error_state_renders")


def test_list_view_renders_with_mock_data():
    """With mocked gh + git, the list view renders the fake issues."""
    with tempfile.TemporaryDirectory() as tmpdir:
        gh = make_fake_gh(tmpdir, mode="ok")
        make_fake_git_repo(tmpdir)
        env = make_env(tmpdir, gh)

        with HarnessWithEnv(APP_PATH, tmpdir, env) as h:
            h.send_init()
            frames = render_until(
                h,
                lambda fr: any("Add bezier draw command" in t for t in h.find_texts(fr)),
            )
            texts = h.find_texts(frames)
            joined = " | ".join(texts)
            assert "Add bezier draw command" in joined, \
                f"Expected first issue title in list view.\nGot: {joined}"
            assert "plexitest/example" in joined, \
                f"Expected repo header to show owner/name.\nGot: {joined}"
            # Issue numbers from the mock should appear.
            assert any("#42" in t for t in texts), "Expected #42 in rendered issue rows"
    print("PASS: test_list_view_renders_with_mock_data")


def test_filter_toggle_open_to_closed():
    """Pressing 'c' triggers a closed-state fetch — app stays alive and re-renders."""
    with tempfile.TemporaryDirectory() as tmpdir:
        gh = make_fake_gh(tmpdir, mode="ok")
        make_fake_git_repo(tmpdir)
        env = make_env(tmpdir, gh)

        with HarnessWithEnv(APP_PATH, tmpdir, env) as h:
            h.send_init()
            # Wait for initial list to land.
            render_until(
                h,
                lambda fr: any("Add bezier draw command" in t for t in h.find_texts(fr)),
            )
            h.send_key("c")
            frames = render_until(
                h,
                lambda fr: any("closed" in t.lower() for t in h.find_texts(fr)),
            )
            texts = h.find_texts(frames)
            joined = " | ".join(texts).lower()
            assert "closed" in joined, f"Expected 'closed' filter to appear.\nGot: {joined}"
    print("PASS: test_filter_toggle_open_to_closed")


if __name__ == "__main__":
    test_lifecycle()
    test_preflight_error_state_renders()
    test_list_view_renders_with_mock_data()
    test_filter_toggle_open_to_closed()
    print("All github-issues tests passed!")
