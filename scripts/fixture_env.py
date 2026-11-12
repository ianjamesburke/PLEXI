"""Test-only subprocess environment for exact-output script fixtures.

`utf8_env` returns a copy of an environment whose locale variables only name
locales this machine supports, so controlled child processes (Perl, shell,
git) never print "Setting locale failed" into output a fixture compares.
Valid inherited settings are preserved; only unsupported ones are replaced.
"""
import os
import subprocess

LOCALE_PREFERENCE = ("C.UTF-8", "en_US.UTF-8")
_ALWAYS_AVAILABLE = frozenset({"c", "posix"})


def _normalize(name):
    """`locale -a` spells UTF-8 as `utf8` on Linux and `UTF-8` on macOS."""
    return name.strip().lower().replace("utf-8", "utf8")


def discover_locales():
    """Normalized names of installed locales, or None when `locale -a` cannot run."""
    try:
        result = subprocess.run(["locale", "-a"], capture_output=True, text=True, check=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        return None
    return {_normalize(line) for line in result.stdout.splitlines() if line.strip()}


def _locale_keys(env):
    return [key for key in env if key == "LANG" or key.startswith("LC_")]


def _supported(name, available):
    return _normalize(name) in _ALWAYS_AVAILABLE or _normalize(name) in available


def _effective_ctype(env):
    for key in ("LC_ALL", "LC_CTYPE", "LANG"):
        if env.get(key):
            return env[key]
    return ""


def utf8_env(base=None, available=None):
    """Copy of `base` (default `os.environ`) with a safe UTF-8 locale.

    `available` is the set from `discover_locales`; pass it to avoid spawning
    `locale -a` or to simulate a platform. None discovers; an empty set means
    discovery found nothing. Outcomes:
    - every set locale variable is supported and the effective one is UTF-8: unchanged
    - else the first of LOCALE_PREFERENCE that is installed becomes LC_ALL and LANG,
      and every other LC_* variable is dropped
    - else (nothing installed, or discovery unavailable) all locale variables are
      dropped, which selects the C locale and never warns
    """
    env = dict(os.environ if base is None else base)
    if available is None:
        available = discover_locales() or set()
    keys = [key for key in _locale_keys(env) if env[key]]
    ctype = _effective_ctype(env)
    if (keys and all(_supported(env[key], available) for key in keys)
            and _normalize(ctype).endswith("utf8")):
        return env
    for key in _locale_keys(env):
        del env[key]
    for candidate in LOCALE_PREFERENCE:
        if _normalize(candidate) in available:
            env["LC_ALL"] = env["LANG"] = candidate
            break
    return env
