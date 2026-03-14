import { existsSync } from "node:fs";

export type ShellLaunchConfig = {
  shellPath: string;
  shellName: string;
  args: string[];
  cwd: string;
  env: Record<string, string>;
};

const SHELL_CANDIDATES = ["/bin/zsh", "/bin/bash", "/bin/sh"];

function resolveFallbackShell() {
  for (const candidate of SHELL_CANDIDATES) {
    if (existsSync(candidate)) {
      return candidate;
    }
  }

  return Bun.which("zsh") || Bun.which("bash") || Bun.which("sh") || "/bin/sh";
}

function normalizeDirectory(pathname: string) {
  return pathname.replace(/\/+$/, "") || "/";
}

export function formatHomeLabel(pathname: string, homeDirectory: string | undefined) {
  if (!homeDirectory) {
    return pathname;
  }

  const normalizedHome = normalizeDirectory(homeDirectory);
  const normalizedPath = normalizeDirectory(pathname);

  if (normalizedPath === normalizedHome) {
    return "~";
  }

  if (normalizedPath.startsWith(`${normalizedHome}/`)) {
    return `~${normalizedPath.slice(normalizedHome.length)}`;
  }

  return pathname;
}

export function expandHomePath(pathname: string | null | undefined, env: Record<string, string | undefined>) {
  if (!pathname || pathname === "~") {
    return env["HOME"] || process.cwd();
  }

  if (pathname.startsWith("~/") && env["HOME"]) {
    return `${env["HOME"]}${pathname.slice(1)}`;
  }

  return pathname;
}

export function resolveShellLaunchConfig(options: {
  cwd?: string | null;
  env?: Record<string, string | undefined>;
} = {}): ShellLaunchConfig {
  const envSource = options.env || process.env;
  const shellPath = envSource["SHELL"] || resolveFallbackShell();
  const shellName = shellPath.split("/").pop() || "shell";
  const cwd = expandHomePath(options.cwd, envSource);

  const args =
    shellName === "fish"
      ? ["--interactive", "--login"]
      : ["-i", "-l"];

  const env: Record<string, string> = {};
  for (const [key, value] of Object.entries(envSource)) {
    if (typeof value === "string") {
      env[key] = value;
    }
  }

  env["TERM"] = "xterm-256color";
  env["COLORTERM"] = env["COLORTERM"] || "truecolor";
  env["TERM_PROGRAM"] = "Plexi";
  env["TERM_PROGRAM_VERSION"] = env["npm_package_version"] || "0.1.0";

  return {
    shellPath,
    shellName,
    args,
    cwd,
    env,
  };
}
