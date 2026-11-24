"""Refuse to run a script that writes a release-channel profile unless it is a disposable account.

Release builds and the installer resolve the profile from the account database, so a
temporary HOME no longer redirects them. Scripts that install or start one must prove
the account is disposable and that no host is live on it.
"""
import os
from pathlib import Path
import socket


def check_disposable(profile_home, environ):
    if (environ.get('GITHUB_ACTIONS') != 'true'
            and environ.get('PLEXI_PACKAGE_TEST_DISPOSABLE_ACCOUNT') != '1'):
        raise SystemExit(f'This script requires a disposable account for {profile_home}. '
                         'On a disposable account set PLEXI_PACKAGE_TEST_DISPOSABLE_ACCOUNT=1.')
    try:
        for profile in profile_home.iterdir():
            if profile.name.startswith('.plexi'):
                path = profile / 'notify.sock'
                if host_listening(path) or host_lock_held(profile / 'host.lock'):
                    raise SystemExit(f'Refusing to run: a live host is using {profile}')
    except OSError as error:
        raise SystemExit(f'Cannot check {profile_home} for an existing host: {error}') from error


def host_listening(path):
    """A stale socket file left by a stopped host refuses connections."""
    if not hasattr(socket, 'AF_UNIX'):
        return False
    with socket.socket(socket.AF_UNIX) as probe:
        probe.settimeout(1)
        try:
            probe.connect(str(path))
        except (FileNotFoundError, NotADirectoryError, ConnectionRefusedError):
            return False
        except OSError:
            return True
    return True


def host_lock_held(path):
    """The host holds an exclusive lock on host.lock for its whole life; the file persists after exit."""
    if os.name == 'nt' or not path.exists():
        return False
    import fcntl
    with path.open('rb') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return True
    return False


def account_home():
    if os.name == 'nt':
        return Path(os.environ['USERPROFILE'])
    import pwd
    return Path(pwd.getpwuid(os.getuid()).pw_dir)
