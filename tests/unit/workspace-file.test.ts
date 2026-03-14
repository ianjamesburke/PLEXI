import { afterEach, describe, expect, test } from "bun:test";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import {
  createWorkspaceStorage,
  readWorkspaceDocument,
  resetWorkspaceStorage,
  writeWorkspaceDocument,
} from "../../src/bun/workspace-file";

const temporaryPaths: string[] = [];

function createTempDir(prefix: string) {
  const directory = mkdtempSync(join(tmpdir(), prefix));
  temporaryPaths.push(directory);
  return directory;
}

afterEach(() => {
  while (temporaryPaths.length > 0) {
    rmSync(temporaryPaths.pop()!, { recursive: true, force: true });
  }
});

describe("workspace storage", () => {
  test("keeps the legacy default workspace path", () => {
    const homeDirectory = createTempDir("plexi-home-");
    const storage = createWorkspaceStorage({
      homeDirectory,
    });

    expect(storage.rootPath).toBe(join(homeDirectory, ".plexi"));
    expect(storage.workspaceFilePath).toBe(join(homeDirectory, ".plexi", "workspaces", "default.json"));
  });

  test("places clean mode in an isolated profile path", () => {
    const homeDirectory = createTempDir("plexi-home-");
    const storage = createWorkspaceStorage({
      homeDirectory,
      profile: "clean",
    });

    expect(storage.rootPath).toBe(join(homeDirectory, ".plexi", "profiles", "clean"));
    expect(storage.workspaceFilePath).toBe(join(
      homeDirectory,
      ".plexi",
      "profiles",
      "clean",
      "workspaces",
      "default.json",
    ));
  });

  test("resets only the clean profile root", () => {
    const homeDirectory = createTempDir("plexi-workspace-");
    const defaultStorage = createWorkspaceStorage({ homeDirectory });
    const cleanStorage = createWorkspaceStorage({ homeDirectory, profile: "clean" });
    const defaultDocument = { workspace: { title: "Default" } };
    const cleanDocument = { workspace: { title: "Clean" } };

    writeWorkspaceDocument(defaultStorage, defaultDocument);
    writeWorkspaceDocument(cleanStorage, cleanDocument);

    resetWorkspaceStorage(cleanStorage);

    expect(existsSync(defaultStorage.workspaceFilePath)).toBe(true);
    expect(existsSync(cleanStorage.workspaceFilePath)).toBe(false);
    expect(readWorkspaceDocument(defaultStorage)).toEqual(defaultDocument);
    expect(readWorkspaceDocument(cleanStorage)).toBeNull();
  });

  test("clean storage behaves like a fresh launch after reset", () => {
    const homeDirectory = createTempDir("plexi-workspace-");
    const defaultStorage = createWorkspaceStorage({ homeDirectory });
    const cleanStorage = createWorkspaceStorage({ homeDirectory, profile: "clean" });
    const defaultDocument = { workspace: { title: "Normal Workspace" } };
    const cleanDocument = { workspace: { title: "First Clean Run" } };

    writeWorkspaceDocument(defaultStorage, defaultDocument);
    writeWorkspaceDocument(cleanStorage, cleanDocument);
    resetWorkspaceStorage(cleanStorage);

    expect(readWorkspaceDocument(cleanStorage)).toBeNull();
    writeWorkspaceDocument(cleanStorage, { workspace: { title: "Second Clean Run" } });

    expect(readWorkspaceDocument(defaultStorage)).toEqual(defaultDocument);
    expect(readWorkspaceDocument(cleanStorage)).toEqual({
      workspace: { title: "Second Clean Run" },
    });
  });
});
