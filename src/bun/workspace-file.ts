import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { homedir } from "node:os";

export type WorkspaceProfile = "default" | "clean";

export type WorkspaceStorage = {
  profile: WorkspaceProfile;
  rootPath: string;
  workspaceFilePath: string;
  source: "disk";
};

type WorkspaceStorageOptions = {
  homeDirectory?: string;
  profile?: WorkspaceProfile;
};

function ensureWorkspaceDirectory(workspaceFilePath: string) {
  mkdirSync(dirname(workspaceFilePath), { recursive: true });
}

export function createWorkspaceStorage(options: WorkspaceStorageOptions = {}): WorkspaceStorage {
  const homeDirectory = options.homeDirectory || homedir();
  const profile = options.profile || "default";

  if (profile === "clean") {
    const rootPath = join(homeDirectory, ".plexi", "profiles", "clean");
    return {
      profile,
      rootPath,
      workspaceFilePath: join(rootPath, "workspaces", "default.json"),
      source: "disk",
    };
  }

  const rootPath = join(homeDirectory, ".plexi");
  return {
    profile: "default",
    rootPath,
    workspaceFilePath: join(rootPath, "workspaces", "default.json"),
    source: "disk",
  };
}

export function resetWorkspaceStorage(storage: WorkspaceStorage) {
  if (storage.profile !== "clean") {
    return;
  }

  rmSync(storage.rootPath, { recursive: true, force: true });
}

export function getWorkspaceFilePath(storage: WorkspaceStorage) {
  return storage.workspaceFilePath;
}

export function readWorkspaceDocument(storage: WorkspaceStorage) {
  if (!existsSync(storage.workspaceFilePath)) {
    return null;
  }

  const raw = readFileSync(storage.workspaceFilePath, "utf8");
  return JSON.parse(raw);
}

export function writeWorkspaceDocument(storage: WorkspaceStorage, document: Record<string, unknown>) {
  ensureWorkspaceDirectory(storage.workspaceFilePath);
  writeFileSync(storage.workspaceFilePath, `${JSON.stringify(document, null, 2)}\n`, "utf8");
  return {
    path: storage.workspaceFilePath,
  };
}
