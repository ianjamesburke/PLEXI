/// Tauri session bridge - replaces Electrobun RPC with Tauri IPC
/// Uses window.__TAURI__.invoke() for commands and polling for output

function hasTauriRuntime() {
  return (
    typeof window !== "undefined" &&
    typeof window.__TAURI__ !== "undefined" &&
    typeof window.__TAURI__.invoke === "function"
  );
}

function createTauriSessionBridge(handlers) {
  const { invoke } = window.__TAURI__.core;
  
  // Poll for output from visible terminals
  // In future: replace with proper event system or WebSocket
  const pollIntervals = new Map();

  return {
    mode: "tauri",
    
    async getBackendInfo() {
      // Not directly supported in Tauri; return synthetic info
      return {
        backend: "pty-process",
        platform: navigator.platform,
        supported: true,
        shellPath: "/bin/zsh",
        shellName: "zsh",
      };
    },

    async openSession(params) {
      const result = await invoke("open_session", {
        panel_id: params.panelId,
        cwd: params.cwd,
        cols: params.cols,
        rows: params.rows,
      });
      
      // Convert response format
      if (result) {
        handlers.onStarted?.({
          panelId: result.panel_id,
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
      return invoke("write_session", {
        panel_id: params.panelId,
        data: params.data,
      });
    },

    async resizeSession(params) {
      return invoke("resize_session", {
        panel_id: params.panelId,
        cols: params.cols,
        rows: params.rows,
      });
    },

    async closeSession(params) {
      // Stop polling for this session
      if (pollIntervals.has(params.panelId)) {
        clearInterval(pollIntervals.get(params.panelId));
        pollIntervals.delete(params.panelId);
      }
      
      return invoke("close_session", {
        panel_id: params.panelId,
      });
    },

    async focusPanel(panelId) {
      // Tell backend this panel is now visible
      // Returns buffered history
      const buffered = await invoke("focus_panel", {
        panel_id: panelId,
      });
      
      if (buffered) {
        handlers.onOutput?.({
          panelId,
          data: buffered,
          seq: 0,
        });
      }

      // Start polling for new output from this panel
      this._startPolling(panelId, handlers);

      return buffered;
    },

    async unfocusPanel(panelId) {
      // Tell backend this panel is now hidden
      // Polling will stop, output queues in ring buffer
      if (pollIntervals.has(panelId)) {
        clearInterval(pollIntervals.get(panelId));
        pollIntervals.delete(panelId);
      }

      return invoke("unfocus_panel", {
        panel_id: panelId,
      });
    },

    _startPolling(panelId, handlers) {
      // Don't start multiple polls for same panel
      if (pollIntervals.has(panelId)) {
        return;
      }

      let lastSeq = 0;
      const interval = setInterval(async () => {
        try {
          const result = await invoke("poll_session_output", {
            panel_id: panelId,
            last_seq: lastSeq,
          });
          
          if (result && result.data) {
            lastSeq = result.seq;
            handlers.onOutput?.({
              panelId,
              data: result.data,
              seq: result.seq,
            });
          }
        } catch (e) {
          console.error(`Error polling ${panelId}:`, e);
        }
      }, 100);
      
      pollIntervals.set(panelId, interval);
    },

    // Workspace storage (filesystem-based in Tauri)
    async getWorkspaceStorageInfo() {
      return {
        profilePath: "~/.plexi",
        configPath: "~/.plexi/config.json",
      };
    },

    async readWorkspaceDocument() {
      // TODO: Implement via tauri-plugin-fs
      return { contexts: [], activeContext: null };
    },

    async writeWorkspaceDocument(params) {
      // TODO: Implement via tauri-plugin-fs
      return {};
    },

    async openExternalUrl(url) {
      // Open in default browser
      if (typeof window !== "undefined" && window.open) {
        window.open(url, "_blank");
      }
      return {};
    },

    async readClipboardText() {
      // TODO: Implement via tauri-plugin-clipboard (if available)
      // For now, use navigator.clipboard if available
      try {
        return await navigator.clipboard.readText();
      } catch (e) {
        console.error("Clipboard read failed:", e);
        return "";
      }
    },

    async writeClipboardText(text) {
      try {
        await navigator.clipboard.writeText(text);
        return {};
      } catch (e) {
        console.error("Clipboard write failed:", e);
        throw e;
      }
    },

    async quitApplication() {
      // Tauri: invoke close command or use tauri-plugin-window
      // For now, just close the window
      if (window.close) {
        window.close();
      }
      return {};
    },

    async reset() {
      // Clear all polling intervals
      for (const interval of pollIntervals.values()) {
        clearInterval(interval);
      }
      pollIntervals.clear();
    },
  };
}

export function createSessionBridge(handlers = {}) {
  if (hasTauriRuntime()) {
    return createTauriSessionBridge(handlers);
  }

  // Fallback to mock for development without Tauri
  console.warn("Tauri runtime not detected, using mock bridge");
  return createMockSessionBridge(handlers);
}
