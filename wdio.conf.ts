import { spawn, execSync, ChildProcess } from "child_process";
import { mkdtempSync, rmSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import type { Options } from "@wdio/types";

let tauriDriver: ChildProcess | null = null;
let tempHome: string | null = null;

function killStaleProcesses() {
  try { execSync("pkill -f tauri-webdriver", { stdio: "ignore" }); } catch {}
  try { execSync("pkill -f 'target/debug/plexi'", { stdio: "ignore" }); } catch {}
}

export const config: Options.Testrunner = {
  runner: "local",
  specs: ["./tests/e2e-binary/**/*.test.ts"],
  maxInstances: 1,
  capabilities: [
    {
      // @ts-expect-error - custom tauri capability
      "tauri:options": {
        application: "./src-tauri/target/debug/plexi",
      },
    },
  ],
  hostname: "localhost",
  port: 4444,
  path: "/",
  framework: "mocha",
  reporters: ["spec"],
  mochaOpts: {
    ui: "bdd",
    timeout: 30000,
  },
  onPrepare() {
    killStaleProcesses();

    // Use a temp HOME so the app starts with a clean ~/.plexi (no saved workspace)
    tempHome = mkdtempSync(join(tmpdir(), "plexi-e2e-"));
    process.env.HOME = tempHome;

    tauriDriver = spawn("tauri-webdriver", ["--port", "4444"], {
      stdio: ["pipe", "pipe", "pipe"],
      env: { ...process.env, HOME: tempHome },
    });

    tauriDriver.stderr?.on("data", (data: Buffer) => {
      const msg = data.toString();
      if (msg.includes("ERROR")) console.error("[tauri-webdriver]", msg.trim());
    });

    // Wait for tauri-webdriver to be ready
    return new Promise<void>((resolve) => {
      setTimeout(resolve, 2000);
    });
  },
  onComplete() {
    if (tauriDriver) {
      tauriDriver.kill();
      tauriDriver = null;
    }
    killStaleProcesses();

    // Clean up temp home
    if (tempHome) {
      try { rmSync(tempHome, { recursive: true, force: true }); } catch {}
      tempHome = null;
    }
  },
};
