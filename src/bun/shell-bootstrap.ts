import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { resolveShellLaunchConfig } from "./shells";

const PLEXI_CWD_OSC = "\u001b]633;PlexiCwd=%s\u0007";

function buildZshBootstrapDirectory(env: Record<string, string>) {
  const sourceDir = env.ZDOTDIR || env.HOME;
  if (!sourceDir) {
    return null;
  }

  const bootstrapDir = mkdtempSync(join(tmpdir(), "plexi-zdotdir-"));
  const relayFile = (name: string, contents: string) => {
    writeFileSync(join(bootstrapDir, name), contents);
  };
  const maybeSource = (name: string) =>
    `[ -f ${JSON.stringify(join(sourceDir, name))} ] && source ${JSON.stringify(join(sourceDir, name))}\n`;

  relayFile(".zshenv", maybeSource(".zshenv"));
  relayFile(".zprofile", maybeSource(".zprofile"));
  relayFile(
    ".zshrc",
    `${maybeSource(".zshrc")}autoload -Uz add-zsh-hook 2>/dev/null\nfunction _plexi_precmd() { printf '${PLEXI_CWD_OSC}' "$PWD"; }\nadd-zsh-hook precmd _plexi_precmd 2>/dev/null || precmd_functions+=(_plexi_precmd)\n_plexi_precmd\n`,
  );
  relayFile(".zlogin", maybeSource(".zlogin"));

  return bootstrapDir;
}

export function applyShellBootstrap(launch: ReturnType<typeof resolveShellLaunchConfig>) {
  if (launch.shellName === "zsh") {
    const bootstrapDir = buildZshBootstrapDirectory(launch.env);
    if (!bootstrapDir) {
      return { launch, bootstrapDir: null };
    }

    return {
      launch: {
        ...launch,
        env: {
          ...launch.env,
          ZDOTDIR: bootstrapDir,
        },
      },
      bootstrapDir,
    };
  }

  if (launch.shellName === "bash") {
    return {
      launch: {
        ...launch,
        env: {
          ...launch.env,
          PROMPT_COMMAND: `printf '${PLEXI_CWD_OSC}' "$PWD";${launch.env.PROMPT_COMMAND ? ` ${launch.env.PROMPT_COMMAND}` : ""}`,
        },
      },
      bootstrapDir: null,
    };
  }

  return { launch, bootstrapDir: null };
}

export function cleanupBootstrapDir(pathname?: string) {
  if (!pathname) {
    return;
  }

  rmSync(pathname, { recursive: true, force: true });
}
