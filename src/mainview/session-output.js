const PLEXI_CWD_SEQUENCE = /\u001b]633;PlexiCwd=([^\u0007\u001b]*)(?:\u0007|\u001b\\)/g;

export function inferHomeDirectory(cwd, cwdLabel) {
  if (!cwd || !cwdLabel || !cwdLabel.startsWith("~")) {
    return null;
  }

  if (cwdLabel === "~") {
    return cwd;
  }

  const suffix = cwdLabel.slice(1);
  if (!cwd.endsWith(suffix)) {
    return null;
  }

  const homePath = cwd.slice(0, cwd.length - suffix.length);
  return homePath || "/";
}

export function formatPathLabel(pathname, homeDirectory = null) {
  if (!pathname) {
    return "~";
  }

  if (!homeDirectory) {
    return pathname;
  }

  if (pathname === homeDirectory) {
    return "~";
  }

  if (pathname.startsWith(`${homeDirectory}/`)) {
    return `~${pathname.slice(homeDirectory.length)}`;
  }

  return pathname;
}

export function extractSessionOutputMetadata(chunk) {
  let nextCwd = null;
  const cleaned = chunk.replace(PLEXI_CWD_SEQUENCE, (_match, cwd) => {
    nextCwd = cwd;
    return "";
  });

  return {
    cleaned,
    cwd: nextCwd,
  };
}
