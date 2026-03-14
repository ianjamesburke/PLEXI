import { describe, expect, test } from "bun:test";
import { expandHomePath, formatHomeLabel, resolveShellLaunchConfig } from "../../src/bun/shells";

describe("shell resolution helpers", () => {
  test("resolveShellLaunchConfig uses login+interactive flags for zsh", () => {
    const config = resolveShellLaunchConfig({
      cwd: "~/project",
      env: {
        HOME: "/tmp/plexi-home",
        SHELL: "/bin/zsh",
        TERM: "xterm-ghostty",
      },
    });

    expect(config.shellPath).toBe("/bin/zsh");
    expect(config.shellName).toBe("zsh");
    expect(config.args).toEqual(["-i", "-l"]);
    expect(config.cwd).toBe("/tmp/plexi-home/project");
    expect(config.env.TERM).toBe("xterm-256color");
    expect(config.env.TERM_PROGRAM).toBe("Plexi");
    expect(config.env.TERM_PROGRAM_VERSION).toBeTruthy();
  });

  test("resolveShellLaunchConfig uses fish-specific flags", () => {
    const config = resolveShellLaunchConfig({
      env: {
        HOME: "/tmp/plexi-home",
        SHELL: "/opt/homebrew/bin/fish",
      },
    });

    expect(config.shellName).toBe("fish");
    expect(config.args).toEqual(["--interactive", "--login"]);
  });

  test("home helpers collapse and expand paths consistently", () => {
    expect(expandHomePath("~/repo", { HOME: "/Users/ian" })).toBe("/Users/ian/repo");
    expect(formatHomeLabel("/Users/ian/repo", "/Users/ian")).toBe("~/repo");
    expect(formatHomeLabel("/tmp/project", "/Users/ian")).toBe("/tmp/project");
  });
});
