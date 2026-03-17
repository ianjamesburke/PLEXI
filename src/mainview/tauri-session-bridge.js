/// Tauri session bridge for desktop IPC
/// Uses native Tauri events for PTY output instead of JS-side polling.
import { createMockSessionBridge } from "./mock-session-bridge.js";

const SESSION_OUTPUT_EVENT = "plexi://session-output";
const SESSION_EXIT_EVENT = "plexi://session-exit";

function hasTauriRuntime() {
  if (typeof window === "undefined") {
    return false;
  }

  return typeof window.__TAURI_INTERNALS__ !== "undefined";
}

function getInvoke() {
  if (typeof window.__TAURI__?.core?.invoke === "function") {
    return window.__TAURI__.core.invoke;
  }

  if (typeof window.__TAURI_INTERNALS__?.invoke === "function") {
    return window.__TAURI_INTERNALS__.invoke;
  }

  throw new Error("Tauri invoke not available");
}

function getListen() {
  if (typeof window.__TAURI__?.event?.listen === "function") {
    return window.__TAURI__.event.listen;
  }

  if (typeof window.__TAURI_INTERNALS__?.event?.listen === "function") {
    return window.__TAURI_INTERNALS__.event.listen;
  }

  throw new Error("Tauri event listen not available");
}

function normalizeInvokeError(error) {
  if (error instanceof Error) {
    return error;
  }

  if (typeof error === "string") {
    return new Error(error);
  }

  const message = error?.message || error?.error || error?.cause || JSON.stringify(error);
  return new Error(message || "Unknown Tauri error");
}

function normalizeSessionOutputPayload(payload) {
  return {
    panelId: payload?.panelId ?? payload?.panel_id ?? "",
    data: payload?.data ?? "",
    seq: payload?.seq,
  };
}

function normalizeSessionExitPayload(payload) {
  return {
    panelId: payload?.panelId ?? payload?.panel_id ?? "",
    exitCode: payload?.exitCode ?? payload?.exit_code ?? null,
  };
}

function createTauriSessionBridge(handlers) {
  const invoke = getInvoke();
  const listen = getListen();
  const sessionListeners = [];

  const registerListener = async (eventName, callback) => {
    const unlisten = await listen(eventName, (event) => {
      callback(event.payload);
    });
    sessionListeners.push(unlisten);
  };

  const startup = Promise.all([
    registerListener(SESSION_OUTPUT_EVENT, (payload) => {
      handlers.onOutput?.(normalizeSessionOutputPayload(payload));
    }),
    registerListener(SESSION_EXIT_EVENT, (payload) => {
      handlers.onExit?.(normalizeSessionExitPayload(payload));
    }),
  ]);

  return {
    mode: "tauri",

    async getBackendInfo() {
      return {
        backend: "pty-process",
        platform: navigator.platform,
        supported: true,
        shellPath: "/bin/zsh",
        shellName: "zsh",
      };
    },

    async openSession(params) {
      await startup;

      let result;
      try {
        result = await invoke("open_session", {
          panelId: params.panelId,
          cwd: params.cwd || null,
          cols: params.cols,
          rows: params.rows,
        });
      } catch (error) {
        throw normalizeInvokeError(error);
      }

      if (result) {
        handlers.onStarted?.({
          panelId: result.panel_id,
          cwd: result.cwd,
          backend: result.backend,
          platform: result.platform,
          shellPath: result.shell_path,
          shellName: result.shell_name,
          cols: result.cols,
          rows: result.rows,
        });
      }

      return result;
    },

    async writeToSession(params) {
      try {
        return await invoke("write_session", {
          panelId: params.panelId,
          data: params.data,
        });
      } catch (error) {
        throw normalizeInvokeError(error);
      }
    },

    async resizeSession(params) {
      try {
        return await invoke("resize_session", {
          panelId: params.panelId,
          cols: params.cols,
          rows: params.rows,
        });
      } catch (error) {
        throw normalizeInvokeError(error);
      }
    },

    async closeSession(params) {
      try {
        return await invoke("close_session", {
          panelId: params.panelId,
        });
      } catch (error) {
        throw normalizeInvokeError(error);
      }
    },

    async getWorkspaceStorageInfo() {
      return {
        profilePath: "~/.plexi",
        configPath: "~/.plexi/config.json",
      };
    },

    async readWorkspaceDocument() {
      return { contexts: [], activeContext: null };
    },

    async writeWorkspaceDocument(_params) {
      return {};
    },

    async openExternalUrl(url) {
      if (typeof window !== "undefined" && window.open) {
        window.open(url, "_blank");
      }
      return {};
    },

    async readClipboardText() {
      try {
        return await navigator.clipboard.readText();
      } catch (error) {
        console.error("Clipboard read failed:", error);
        return "";
      }
    },

    async writeClipboardText(text) {
      try {
        await navigator.clipboard.writeText(text);
        return {};
      } catch (error) {
        console.error("Clipboard write failed:", error);
        throw error;
      }
    },

    async quitApplication() {
      if (window.close) {
        window.close();
      }
      return {};
    },

    async reset() {
      for (const unlisten of sessionListeners.splice(0)) {
        await unlisten?.();
      }
    },
  };
}

export function createSessionBridge(handlers = {}) {
  if (hasTauriRuntime()) {
    return createTauriSessionBridge(handlers);
  }

  console.warn("Tauri runtime not detected, using mock bridge");
  return createMockSessionBridge(handlers);
}
