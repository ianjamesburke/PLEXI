import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { LocalSessionManager } from "../../src/bun/session-manager";

const temporaryPaths = [];

function createTempDir(prefix) {
  const directory = mkdtempSync(join(tmpdir(), prefix));
  temporaryPaths.push(directory);
  return directory;
}

async function waitFor(check, timeoutMs = 5000) {
  const startedAt = Date.now();

  while (Date.now() - startedAt < timeoutMs) {
    if (check()) {
      return;
    }

    await Bun.sleep(40);
  }

  throw new Error("Timed out waiting for PTY session output");
}

afterEach(() => {
  while (temporaryPaths.length > 0) {
    rmSync(temporaryPaths.pop(), { recursive: true, force: true });
  }
});

describe("LocalSessionManager", () => {
  test("executes commands inside a PTY-backed zsh session", async () => {
    if (process.platform === "win32") {
      return;
    }

    const homeDir = createTempDir("plexi-home-");
    const workingDir = createTempDir("plexi-cwd-");
    const resolvedWorkingDir = realpathSync(workingDir);
    const output = [];
    const exits = [];
    const manager = new LocalSessionManager(
      {
        onOutput(message) {
          output.push(message.data);
        },
        onExit(message) {
          exits.push(message.exitCode);
        },
      },
      {
        ...process.env,
        HOME: homeDir,
        SHELL: "/bin/zsh",
      },
    );

    const started = await manager.openSession({
      panelId: "panel-1",
      cwd: workingDir,
      cols: 100,
      rows: 30,
    });

    await Bun.sleep(300);
    manager.writeToSession("panel-1", 'printf "pwd=%s\\n" "$PWD"\r');
    manager.writeToSession("panel-1", "exit\r");

    await waitFor(() => output.join("").includes(`pwd=${resolvedWorkingDir}`));
    await waitFor(() => exits.length === 1);

    expect(started.shellName).toBe("zsh");
    expect(started.cwd).toBe(workingDir);
    expect(output.join("")).toContain(`pwd=${resolvedWorkingDir}`);
    expect(exits[0]).toBe(0);
  });

  test("loads zsh startup config from HOME", async () => {
    if (process.platform === "win32") {
      return;
    }

    const homeDir = createTempDir("plexi-zsh-home-");
    const output = [];
    const manager = new LocalSessionManager(
      {
        onOutput(message) {
          output.push(message.data);
        },
      },
      {
        ...process.env,
        HOME: homeDir,
        SHELL: "/bin/zsh",
      },
    );

    writeFileSync(join(homeDir, ".zshrc"), "export PLEXI_ZSHRC_LOADED=1\n");

    await manager.openSession({
      panelId: "panel-rc",
      cwd: homeDir,
    });

    await Bun.sleep(300);
    manager.writeToSession("panel-rc", 'printf "rc=%s\\n" "$PLEXI_ZSHRC_LOADED"\r');
    manager.writeToSession("panel-rc", "exit\r");

    await waitFor(() => output.join("").includes("rc=1"));
    expect(output.join("")).toContain("rc=1");
  });

  test("interrupts a foreground process with control-c", async () => {
    if (process.platform === "win32") {
      return;
    }

    const output = [];
    const manager = new LocalSessionManager(
      {
        onOutput(message) {
          output.push(message.data);
        },
      },
      {
        ...process.env,
        SHELL: "/bin/zsh",
      },
    );

    await manager.openSession({
      panelId: "panel-interrupt",
      cols: 100,
      rows: 30,
    });

    await Bun.sleep(300);
    manager.writeToSession("panel-interrupt", "sleep 10\r");
    await Bun.sleep(250);
    manager.writeToSession("panel-interrupt", "\u0003");
    await Bun.sleep(150);
    manager.writeToSession("panel-interrupt", 'printf "interrupt=ok\\n"\r');
    manager.writeToSession("panel-interrupt", "exit\r");

    await waitFor(() => output.join("").includes("interrupt=ok"), 4000);
    expect(output.join("")).toContain("interrupt=ok");
  });
});
