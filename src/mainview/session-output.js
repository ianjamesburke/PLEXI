const PLEXI_CWD_SEQUENCE = /\u001b]633;PlexiCwd=([^\u0007\u001b]*)(?:\u0007|\u001b\\)/g;
const ANSI_SEQUENCE = /\u001b(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~]|\][^\u0007\u001b]*(?:\u0007|\u001b\\))/g;
const NON_PRINTABLE = /[\u0000-\u0008\u000b-\u001f\u007f]/g;

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

export function sanitizeTerminalPreview(chunk) {
  return String(chunk || "")
    .replace(ANSI_SEQUENCE, "")
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "\n")
    .replace(NON_PRINTABLE, "")
    .trimEnd();
}
