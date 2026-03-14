import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

type BunPtyModule = typeof import("bun-pty");

let bunPtyModulePromise: Promise<BunPtyModule> | null = null;

function getBunPtyLibFilenames() {
  if (process.platform === "darwin") {
    return process.arch === "arm64"
      ? ["librust_pty_arm64.dylib", "librust_pty.dylib"]
      : ["librust_pty.dylib"];
  }

  if (process.platform === "win32") {
    return ["rust_pty.dll"];
  }

  return process.arch === "arm64"
    ? ["librust_pty_arm64.so", "librust_pty.so"]
    : ["librust_pty.so"];
}

function resolveBunPtyLibraryPath() {
  const configured = process.env.BUN_PTY_LIB;
  if (configured && existsSync(configured)) {
    return configured;
  }

  const here = dirname(fileURLToPath(import.meta.url));
  const filenames = getBunPtyLibFilenames();
  const candidates = [];

  for (const filename of filenames) {
    candidates.push(join(here, "..", "native", filename));
    candidates.push(join(here, "native", filename));
    candidates.push(join(process.cwd(), "native", filename));
    candidates.push(join(process.cwd(), "node_modules", "bun-pty", "rust-pty", "target", "release", filename));
  }

  return candidates.find((candidate) => existsSync(candidate)) || null;
}

export async function loadBunPty() {
  if (!bunPtyModulePromise) {
    const libraryPath = resolveBunPtyLibraryPath();
    if (libraryPath) {
      process.env.BUN_PTY_LIB = libraryPath;
    }
    bunPtyModulePromise = import("bun-pty");
  }

  return bunPtyModulePromise;
}
