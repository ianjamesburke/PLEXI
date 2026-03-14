import { createContextRecord, makeDefaultState } from "../shared/workspace-state.js";
import { STORAGE_KEY } from "./app-constants.js";
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

export async function hydrateWorkspaceState(sessionBridge) {
  if (!sessionBridge || sessionBridge.mode !== "live") {
    return {
      state: loadWorkspaceState(),
      storage: await sessionBridge?.getWorkspaceStorageInfo?.(),
    };
  }

  const [storage, stored] = await Promise.all([
    sessionBridge.getWorkspaceStorageInfo(),
    sessionBridge.readWorkspaceDocument(),
  ]);

  if (!stored?.document) {
    return {
      state: bootDefaultState(),
      storage,
    };
  }

  return {
    state: parseStoredState(stored.document),
    storage,
  };
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
  window.localStorage.setItem(STORAGE_KEY, JSON.stringify(snapshot.document));

  if (sessionBridge?.mode === "live") {
    return sessionBridge.writeWorkspaceDocument({
      document: snapshot.document,
    });
  }

  return sessionBridge?.getWorkspaceStorageInfo?.() || {
    path: "Browser storage",
    source: "browser",
  };
}
