import { createContextRecord, makeDefaultState } from "../shared/workspace-state.js";
import { DEFAULT_WORKSPACE_NAME, STORAGE_KEY } from "./app-constants.js";
import {
  deserializeWorkspaceDocument,
  formatWorkspaceDocumentJson,
  migrateLegacyWorkspaceState,
  serializeWorkspaceDocument,
} from "../shared/workspace-document.js";

export function bootDefaultState() {
  const nextState = makeDefaultState();
  createContextRecord(nextState, "");
  nextState.lastAction = "Ready";
  return nextState;
}

function parseStoredState(parsed) {
  const nextState = parsed?.workspace
    ? deserializeWorkspaceDocument(parsed)
    : migrateLegacyWorkspaceState(parsed);

  if (nextState.contexts.length === 0) {
    createContextRecord(nextState, "");
  }

  return nextState;
}

export function loadWorkspaceState() {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);

    if (!raw) {
      return bootDefaultState();
    }

    return parseStoredState(JSON.parse(raw));
  } catch (_error) {
    return bootDefaultState();
  }
}

function hasDiskBridge(sessionBridge) {
  return sessionBridge && (sessionBridge.mode === "live" || sessionBridge.mode === "tauri");
}

export async function hydrateWorkspaceState(sessionBridge) {
  if (!hasDiskBridge(sessionBridge)) {
    return {
      state: loadWorkspaceState(),
      storage: await sessionBridge?.getWorkspaceStorageInfo?.(),
    };
  }

  const [storage, stored] = await Promise.all([
    sessionBridge.getWorkspaceStorageInfo(),
    sessionBridge.readWorkspaceDocument({ name: DEFAULT_WORKSPACE_NAME }),
  ]);

  if (!stored?.document) {
    // No disk file (first launch, user deleted ~/.plexi, or corrupt JSON).
    // In Tauri mode we do NOT fall back to localStorage — that would silently
    // restore stale state and make deleting ~/.plexi appear to have no effect.
    // localStorage is a write-through cache only; disk is the source of truth.
    return {
      state: bootDefaultState(),
      storage,
      warning: stored?.warning ?? null,
    };
  }

  try {
    return {
      state: parseStoredState(stored.document),
      storage,
    };
  } catch (error) {
    // Document parsed as JSON but failed to deserialize into app state.
    console.error("Workspace file structure is invalid, starting fresh:", error);
    return {
      state: bootDefaultState(),
      storage,
      warning: "Workspace file structure was invalid and has been reset to defaults.",
    };
  }
}

export function getWorkspaceSnapshot(state) {
  const document = serializeWorkspaceDocument(state);
  return {
    document,
    json: formatWorkspaceDocumentJson(state),
  };
}

export async function saveWorkspaceState(state, sessionBridge) {
  const snapshot = getWorkspaceSnapshot(state);

  // Always keep localStorage as a fast fallback
  window.localStorage.setItem(STORAGE_KEY, JSON.stringify(snapshot.document));

  if (hasDiskBridge(sessionBridge)) {
    await sessionBridge.writeWorkspaceDocument({
      name: DEFAULT_WORKSPACE_NAME,
      document: snapshot.document,
    });
    return sessionBridge.getWorkspaceStorageInfo();
  }

  return sessionBridge?.getWorkspaceStorageInfo?.() || {
    path: "Browser storage",
    source: "browser",
  };
}
