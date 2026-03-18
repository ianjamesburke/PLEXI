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
          homeDir: result.home_dir,
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
      try {
        const paths = await invoke("get_plexi_paths", {});
        return {
          profilePath: paths.base,
          configPath: paths.config,
          workspacesPath: paths.workspaces,
          source: "disk",
        };
      } catch (error) {
        console.error("getWorkspaceStorageInfo failed:", error);
        return {
          profilePath: "~/.plexi",
          configPath: "~/.plexi/config.json",
          source: "disk",
        };
      }
    },

    async readWorkspaceDocument(params) {
      const name = params?.name;
      let raw;
      try {
        raw = await invoke("read_workspace", { name });
      } catch (error) {
        console.error(`Failed to read workspace "${name}":`, error);
        return { document: null };
      }

      if (!raw) {
        return { document: null };
      }

      try {
        return { document: JSON.parse(raw) };
      } catch (parseError) {
        console.error(
          `Workspace file "${name}.json" contains invalid JSON:`,
          parseError.message
        );

        // Rename the corrupt file so the next save doesn't overwrite it.
        // The user can find it, fix the typo, and rename it back.
        let backupName = null;
        try {
          backupName = await invoke("backup_workspace", { name });
          console.warn(`Corrupt workspace backed up to ~/.plexi/workspaces/${backupName}`);
        } catch (backupError) {
          console.error("Could not back up corrupt workspace file:", backupError);
        }

        return {
          document: null,
          warning: backupName
            ? `Workspace file had invalid JSON — backed up to ${backupName}. Fix it and rename to ${name}.json to restore.`
            : "Workspace file had invalid JSON and could not be backed up — starting fresh.",
        };
      }
    },

    async writeWorkspaceDocument(params) {
      const name = params?.name;
      try {
        const contents = JSON.stringify(params.document, null, 2);
        await invoke("write_workspace", { name, contents });
        return {};
      } catch (error) {
        console.error("writeWorkspaceDocument failed:", error);
        throw normalizeInvokeError(error);
      }
    },

    async listWorkspaces() {
      try {
        return await invoke("list_workspaces", {});
      } catch (error) {
        console.error("listWorkspaces failed:", error);
        return [];
      }
    },

    async readConfig() {
      let raw;
      try {
        raw = await invoke("read_config", {});
      } catch (error) {
        console.error("Failed to read ~/.plexi/config.json:", error);
        return null;
      }

      if (!raw) {
        return null;
      }

      try {
        return JSON.parse(raw);
      } catch (error) {
        console.error(
          "~/.plexi/config.json contains invalid JSON:",
          error.message,
          "\nFalling back to defaults. Fix the JSON syntax and restart."
        );
        return null;
      }
    },

    async writeConfig(config) {
      try {
        const contents = JSON.stringify(config, null, 2);
        await invoke("write_config", { contents });
        return {};
      } catch (error) {
        console.error("writeConfig failed:", error);
        throw normalizeInvokeError(error);
      }
    },

    async openExternalUrl(url) {
      const invoke = getInvoke();
      await invoke("plugin:shell|open", { path: url });
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
