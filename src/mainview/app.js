import {
  DIRECTIONS,
  PANEL_TYPES,
  adjustZoom,
  clone,
  closePanelRecord,
  createContextRecord,
  createPanelRecord,
  ensureActivePanel,
  focusDirectionalPanel,
  focusPanel,
  getActivePanel,
  getBounds,
  getVisiblePanels,
  movePanelRecord,
  panCamera,
  renameContextRecord,
  resetViewport,
  setContextIndex,
  toggleMode,
} from "../shared/workspace-state.js";
import { CONTEXT_COMMANDS, WORKSPACE_COMMANDS, isWorkspaceCommand } from "../shared/commands.js";
import { getDisplayContextLabel } from "../shared/workspace-document.js";
import { resolveKeybind } from "../shared/keybinds.js";
import { createSessionBridge } from "./session-bridge.js";
import { applyPlatformClasses, MAX_BUFFER_CHARS } from "./app-constants.js";
import { dom } from "./dom.js";
import { APP_KEYBINDS } from "./keybind-config.js";
import { resolveTerminalKeybind } from "./terminal-shortcuts.js";
import {
  extractSessionOutputMetadata,
  formatPathLabel,
  inferHomeDirectory,
} from "./session-output.js";
import {
  bootDefaultState,
  getWorkspaceSnapshot,
  hydrateWorkspaceState,
  loadWorkspaceState,
  saveWorkspaceState,
} from "./workspace-storage.js";
import {
  createTerminalRuntime,
  ensureTerminalFont,
  ensureXtermAssets,
  getTerminalProfile,
  getXtermStatus,
  setXtermError,
} from "./xterm-runtime.js";

let terminalRuntime = null;
const panelBuffers = new Map();
const panelMeta = new Map();
const panelSessions = new Set();
let backendInfo = null;
let homeDirectory = null;
let state = bootDefaultState();
const uiState = {
  workspaceStoragePath: "Resolving workspace file…",
  workspaceStorageSource: "browser",
  workspaceInspectorVisible: false,
  contextModalOpen: false,
  contextModalMode: "create",
  contextModalIndex: null,
  toastTimer: null,
};

applyPlatformClasses();

const sessionBridge = createSessionBridge({
  onStarted(message) {
    panelMeta.set(message.panelId, {
      shellName: message.shellName,
      shellPath: message.shellPath,
      backend: message.backend,
    });
    homeDirectory = homeDirectory || inferHomeDirectory(message.cwd, message.cwdLabel);
    const panel = state.panels.find((item) => item.id === message.panelId);
    if (!panel) {
      return;
    }

    panel.cwd = message.cwd;
    panel.cwdLabel = message.cwdLabel;
    saveState();
    render();
  },
  onOutput(message) {
    const { cleaned, cwd } = extractSessionOutputMetadata(message.data);

    if (cwd) {
      updatePanelDirectory(message.panelId, cwd);
    }

    if (!cleaned) {
      return;
    }

    appendPanelBuffer(message.panelId, cleaned);

    if (terminalRuntime?.panel?.id === message.panelId) {
      terminalRuntime.terminal.write(cleaned);
    }
  },
  onExit(message) {
    panelSessions.delete(message.panelId);
    appendPanelBuffer(message.panelId, `\r\n[session exited ${message.exitCode}]\r\n`);

    if (terminalRuntime?.panel?.id === message.panelId) {
      terminalRuntime.terminal.write(`\r\n[session exited ${message.exitCode}]\r\n`);
    }
  },
  onError(message) {
    panelSessions.delete(message.panelId);
    appendPanelBuffer(message.panelId, `\r\n[session error] ${message.message}\r\n`);
    setLastAction(`Session error in ${message.panelId}`);
    saveState();
    render();
  },
  onWorkspaceCommand(command) {
    runCommand(command);
  },
  onClear(panelId) {
    clearPanelBuffer(panelId);
    if (terminalRuntime?.panel?.id === panelId) {
      terminalRuntime.terminal.clear();
    }
  },
});

if (sessionBridge.mode !== "live") {
  state = loadWorkspaceState();
}

function saveState() {
  const snapshot = clone(state);
  void saveWorkspaceState(snapshot, sessionBridge)
    .then(updateWorkspaceStorage)
    .catch(() => {
      showToast("Workspace save failed");
    });
}

function showToast(message) {
  if (!dom.toastLayer || !message) {
    return;
  }

  dom.toastLayer.innerHTML = `<div class="toast">${message}</div>`;
  dom.toastLayer.classList.add("is-visible");

  if (uiState.toastTimer) {
    window.clearTimeout(uiState.toastTimer);
  }

  uiState.toastTimer = window.setTimeout(() => {
    dom.toastLayer.classList.remove("is-visible");
  }, 2200);
}

function updateWorkspaceStorage(storage) {
  if (!storage) {
    return;
  }

  uiState.workspaceStoragePath = storage.path;
  uiState.workspaceStorageSource = storage.source;
}

function setLastAction(label, { toast = true } = {}) {
  state.lastAction = label;
  if (toast) {
    showToast(label);
  }
}

function getContextCount() {
  return state.contexts.length;
}

function getContextLabel(index) {
  return state.contexts[index]?.label || "";
}

function getActiveContextLabel() {
  return getContextLabel(state.activeContextIndex);
}

function formatContextLabel(label, index = state.activeContextIndex) {
  return getDisplayContextLabel(label, index);
}

function formatContextStatusLabel(label, index = state.activeContextIndex) {
  return formatContextLabel(label, index);
}

function syncViewportMetrics() {
  const candidates = [
    window.visualViewport?.height,
    window.innerHeight,
    document.documentElement.clientHeight,
    document.body?.clientHeight,
  ].filter((value) => Number.isFinite(value) && value > 0);
  const viewportHeight = Math.round(candidates.length > 0 ? Math.min(...candidates) : 0);

  if (viewportHeight > 0) {
    document.documentElement.style.setProperty("--app-height", `${viewportHeight}px`);
  }
}

function appendPanelBuffer(panelId, chunk) {
  const previous = panelBuffers.get(panelId) || "";
  const next = `${previous}${chunk}`;
  panelBuffers.set(
    panelId,
    next.length > MAX_BUFFER_CHARS ? next.slice(-MAX_BUFFER_CHARS) : next,
  );
}

function clearPanelBuffer(panelId) {
  panelBuffers.set(panelId, "");
}

function replayBuffer(runtime) {
  runtime.terminal.clear();
  runtime.terminal.write(panelBuffers.get(runtime.panel.id) || "");
}

function updatePanelDirectory(panelId, cwd) {
  const panel = state.panels.find((item) => item.id === panelId);
  if (!panel || panel.cwd === cwd) {
    return;
  }

  panel.cwd = cwd;
  panel.cwdLabel = formatPathLabel(cwd, homeDirectory);
  saveState();

  if (terminalRuntime?.panel?.id === panelId) {
    render();
  }
}

async function copySelection(runtime) {
  const selection = runtime?.terminal?.getSelection?.();

  if (!selection) {
    return;
  }

  try {
    await navigator.clipboard.writeText(selection);
    setLastAction("Selection copied");
  } catch (_error) {
    setLastAction("Clipboard copy failed");
  }

  saveState();
  render();
}

async function pasteClipboard(runtime) {
  try {
    const text = await navigator.clipboard.readText();

    if (!text) {
      return;
    }

    await sessionBridge.writeToSession({
      panelId: runtime.panel.id,
      data: text,
    });
    setLastAction("Clipboard pasted");
  } catch (_error) {
    setLastAction("Clipboard paste failed");
  }

  saveState();
  render();
}

async function ensurePanelSession(panel) {
  if (!panel || panel.type !== "terminal" || panelSessions.has(panel.id)) {
    return;
  }

  panelSessions.add(panel.id);
  clearPanelBuffer(panel.id);

  try {
    await sessionBridge.openSession({
      panelId: panel.id,
      cwd: panel.cwd,
      cols: terminalRuntime?.terminal?.cols || 80,
      rows: terminalRuntime?.terminal?.rows || 24,
    });
  } catch (error) {
    panelSessions.delete(panel.id);
    appendPanelBuffer(panel.id, `\r\n[session failed] ${error.message}\r\n`);
    setLastAction(`Session failed for ${panel.title}`);
    saveState();
    render();
  }
}

function ensurePanelSessions() {
  state.panels.forEach((panel) => {
    void ensurePanelSession(panel);
  });
}

function closePanelSession(panelId) {
  panelSessions.delete(panelId);
  panelMeta.delete(panelId);
  panelBuffers.delete(panelId);
  void sessionBridge.closeSession({ panelId });
}

function disposeRuntime() {
  if (!terminalRuntime) {
    return;
  }

  terminalRuntime.dispose();
  terminalRuntime = null;
}

async function mountActiveTerminal(activePanel) {
  if (!activePanel || activePanel.type !== "terminal") {
    disposeRuntime();
    dom.terminalMount.innerHTML = "";
    return;
  }

  try {
    await ensureXtermAssets();
    await ensureTerminalFont();
  } catch (error) {
    setXtermError();
    if (dom.engineLabel) {
      dom.engineLabel.textContent = "xterm.js failed";
    }
    dom.terminalMount.textContent = String(error);
    dom.terminalMount.classList.add("terminal-mount--error");
    return;
  }

  if (dom.engineLabel && getXtermStatus() === "ready") {
    dom.engineLabel.textContent = "xterm.js ready";
  }

  dom.terminalMount.classList.remove("terminal-mount--loading", "terminal-mount--error");
  if (terminalRuntime?.panel?.id !== activePanel.id) {
    disposeRuntime();
    dom.terminalMount.innerHTML = "";
    terminalRuntime = createTerminalRuntime({
      panel: activePanel,
      mountNode: dom.terminalMount,
      onData(runtime, rawData) {
        void sessionBridge.writeToSession({
          panelId: runtime.panel.id,
          data: rawData,
        });
      },
      onShortcut(event, runtime) {
        if (handleShortcutKeydown(event)) {
          return false;
        }

        const match = resolveTerminalKeybind(event, {
          hasSelection: Boolean(runtime.terminal.getSelection()),
        });

        if (!match) {
          return true;
        }

        if (match.action.name === "copy_to_clipboard") {
          event.preventDefault();
          void copySelection(runtime);
          return false;
        }

        if (match.action.name === "paste_from_clipboard") {
          event.preventDefault();
          void pasteClipboard(runtime);
          return false;
        }

        if (match.consume) {
          event.preventDefault();
          return false;
        }

        return true;
      },
      onResize(runtime) {
        void sessionBridge.resizeSession({
          panelId: runtime.panel.id,
          cols: runtime.terminal.cols,
          rows: runtime.terminal.rows,
        });
      },
      replayBuffer,
    });
    return;
  }

  terminalRuntime.panel = activePanel;
  replayBuffer(terminalRuntime);
  void sessionBridge.resizeSession({
    panelId: activePanel.id,
    cols: terminalRuntime.terminal.cols,
    rows: terminalRuntime.terminal.rows,
  });
  terminalRuntime.terminal.focus();
}

function updateEngineLabel(activePanel) {
  if (getXtermStatus() === "error") {
    return;
  }

  const activeMeta = activePanel ? panelMeta.get(activePanel.id) : null;

  if (activeMeta?.shellName) {
    if (dom.engineLabel) {
      dom.engineLabel.textContent = `xterm.js + ${activeMeta.shellName}`;
    }
    return;
  }

  if (backendInfo?.shellName) {
    if (dom.engineLabel) {
      dom.engineLabel.textContent = `xterm.js + ${backendInfo.shellName}`;
    }
    return;
  }

  if (backendInfo?.backend === "mock") {
    if (dom.engineLabel) {
      dom.engineLabel.textContent = "xterm.js + mock shell";
    }
    return;
  }

  if (dom.engineLabel) {
    dom.engineLabel.textContent = getXtermStatus() === "ready" ? "xterm.js ready" : "Loading xterm.js";
  }
}

function renderContextModal() {
  if (!dom.contextModal) {
    return;
  }

  dom.contextModal.classList.toggle("is-hidden", !uiState.contextModalOpen);

  if (!uiState.contextModalOpen) {
    return;
  }

  const isRename = uiState.contextModalMode === "rename";
  const submitLabel = isRename ? "Save changes" : "Create context";

  if (dom.contextModalTitle) {
    dom.contextModalTitle.textContent = isRename ? "Edit context" : "New context";
  }
  if (dom.contextSubmitButton) {
    dom.contextSubmitButton.textContent = submitLabel;
  }
}

function openContextModal({ mode, index = null }) {
  const existing = index === null ? null : state.contexts[index];
  uiState.contextModalOpen = true;
  uiState.contextModalMode = mode;
  uiState.contextModalIndex = index;
  renderContextModal();

  if (dom.contextNameInput) {
    dom.contextNameInput.value = existing
      ? formatContextLabel(existing.label, index)
      : `Context ${getContextCount() + 1}`;
  }

  window.requestAnimationFrame(() => {
    dom.contextNameInput?.focus();
    dom.contextNameInput?.select();
  });
}

function closeContextModal() {
  uiState.contextModalOpen = false;
  uiState.contextModalMode = "create";
  uiState.contextModalIndex = null;
  renderContextModal();
}

function toggleWorkspaceJson() {
  uiState.workspaceInspectorVisible = !uiState.workspaceInspectorVisible;
  setLastAction(uiState.workspaceInspectorVisible ? "Workspace JSON opened" : "Workspace JSON closed");
  render();
}

function submitContextModal() {
  const label = dom.contextNameInput?.value?.trim();

  if (!label) {
    dom.contextNameInput?.focus();
    return;
  }

  if (uiState.contextModalMode === "rename" && Number.isInteger(uiState.contextModalIndex)) {
    renameContextRecord(state, uiState.contextModalIndex, label);
    setLastAction(`Context renamed to ${label}`);
  } else {
    createContextRecord(state, label);
    setLastAction(`Context ${label} created`);
  }

  closeContextModal();
  saveState();
  render();
}

function switchToContext(index) {
  if (index < 0 || index >= getContextCount()) {
    return false;
  }
  setContextIndex(state, index);
  setLastAction(`Context ${formatContextLabel(getActiveContextLabel())}`);
  saveState();
  render();
  return true;
}

function createTerminal(direction, cwd = null, cwdLabel = null) {
  const panel = createPanelRecord(state, { direction, cwd, cwdLabel });
  clearPanelBuffer(panel.id);
  void ensurePanelSession(panel);
  setLastAction(
    direction === DIRECTIONS.down
      ? `${panel.title} created below`
      : `${panel.title} created to the right`,
  );
  state.mode = "focus";
}

function closeActiveTerminal() {
  const activePanel = getActivePanel(state);

  if (!activePanel) {
    setLastAction("No terminal to close");
    return;
  }

  const removed = closePanelRecord(state, activePanel.id);

  if (removed) {
    if (terminalRuntime?.panel?.id === removed.id) {
      disposeRuntime();
      dom.terminalMount.innerHTML = "";
    }
    closePanelSession(removed.id);
    setLastAction(`${removed.title} closed`);
  }

  if (getVisiblePanels(state).length === 0) {
    createTerminal(
      DIRECTIONS.right,
      removed?.cwd || "~",
      removed?.cwdLabel || removed?.cwd || "~",
    );
  }
}

function handleDirectionalFocus(direction) {
  const panel = focusDirectionalPanel(state, direction);

  if (panel) {
    setLastAction(`Focused ${panel.title}`);
    return;
  }

  setLastAction("No terminal in that direction");
}

function toggleShortcuts() {
  state.shortcutsVisible = !state.shortcutsVisible;
  setLastAction(state.shortcutsVisible ? "Keyboard reference open" : "Keyboard reference closed");
}

function handleShortcutKeydown(event) {
  if (event.type !== "keydown" || event.defaultPrevented) {
    return false;
  }
  const isArrowKey = ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key);
  const match = resolveKeybind(event, APP_KEYBINDS);

  if (match) {
    event.preventDefault();
    const actionToCommand = {
      close_terminal: WORKSPACE_COMMANDS.closeTerminal,
      context_1: WORKSPACE_COMMANDS.context1,
      context_2: WORKSPACE_COMMANDS.context2,
      context_3: WORKSPACE_COMMANDS.context3,
      context_4: WORKSPACE_COMMANDS.context4,
      context_5: WORKSPACE_COMMANDS.context5,
      context_6: WORKSPACE_COMMANDS.context6,
      context_7: WORKSPACE_COMMANDS.context7,
      context_8: WORKSPACE_COMMANDS.context8,
      context_9: WORKSPACE_COMMANDS.context9,
      focus_down: WORKSPACE_COMMANDS.focusDown,
      focus_left: WORKSPACE_COMMANDS.focusLeft,
      focus_right: WORKSPACE_COMMANDS.focusRight,
      focus_up: WORKSPACE_COMMANDS.focusUp,
      new_terminal_down: WORKSPACE_COMMANDS.newTerminalDown,
      new_terminal_right: WORKSPACE_COMMANDS.newTerminalRight,
      save_workspace: WORKSPACE_COMMANDS.saveWorkspace,
      toggle_overview: WORKSPACE_COMMANDS.toggleOverview,
      toggle_shortcuts: WORKSPACE_COMMANDS.toggleShortcuts,
      toggle_sidebar: WORKSPACE_COMMANDS.toggleSidebar,
      zoom_in: WORKSPACE_COMMANDS.zoomIn,
      zoom_out: WORKSPACE_COMMANDS.zoomOut,
    };
    const command = actionToCommand[match.action.name];
    if (command) {
      runCommand(command);
      return true;
    }
  }

  if (state.mode === "overview" && (event.metaKey || event.ctrlKey) && event.shiftKey && isArrowKey) {
    event.preventDefault();

    const direction =
      event.key === "ArrowLeft"
        ? DIRECTIONS.left
        : event.key === "ArrowRight"
          ? DIRECTIONS.right
          : event.key === "ArrowUp"
            ? DIRECTIONS.up
            : DIRECTIONS.down;

    const moved = movePanelRecord(state, state.activePanelId, direction);

    if (moved) {
      setLastAction(`${moved.title} repositioned`);
      saveState();
      render();
    }

    return true;
  }

  if (state.mode === "overview" && isArrowKey && !(event.metaKey || event.ctrlKey)) {
    event.preventDefault();

    const distance = 120;
    if (event.key === "ArrowLeft") {
      panCamera(state, distance, 0);
    } else if (event.key === "ArrowRight") {
      panCamera(state, -distance, 0);
    } else if (event.key === "ArrowUp") {
      panCamera(state, 0, distance);
    } else if (event.key === "ArrowDown") {
      panCamera(state, 0, -distance);
    }
    setLastAction("Overview moved");
    saveState();
    render();
    return true;
  }

  return false;
}

function runCommand(command) {
  if (!isWorkspaceCommand(command)) {
    return;
  }

  const contextCount = getContextCount();

  switch (command) {
    case WORKSPACE_COMMANDS.newTerminalRight:
      createTerminal(DIRECTIONS.right);
      break;
    case WORKSPACE_COMMANDS.newTerminalDown:
      createTerminal(DIRECTIONS.down);
      break;
    case WORKSPACE_COMMANDS.closeTerminal:
      closeActiveTerminal();
      break;
    case WORKSPACE_COMMANDS.toggleOverview:
      setLastAction(toggleMode(state) === "overview" ? "Overview opened" : "Focus mode");
      break;
    case WORKSPACE_COMMANDS.toggleSidebar:
      state.sidebarVisible = !state.sidebarVisible;
      setLastAction(state.sidebarVisible ? "Sidebar shown" : "Sidebar hidden");
      break;
    case WORKSPACE_COMMANDS.toggleShortcuts:
    case WORKSPACE_COMMANDS.showShortcuts:
      toggleShortcuts();
      break;
    case WORKSPACE_COMMANDS.saveWorkspace:
      saveState();
      setLastAction("Workspace saved");
      break;
    case WORKSPACE_COMMANDS.focusLeft:
      handleDirectionalFocus(DIRECTIONS.left);
      break;
    case WORKSPACE_COMMANDS.focusRight:
      handleDirectionalFocus(DIRECTIONS.right);
      break;
    case WORKSPACE_COMMANDS.focusUp:
      handleDirectionalFocus(DIRECTIONS.up);
      break;
    case WORKSPACE_COMMANDS.focusDown:
      handleDirectionalFocus(DIRECTIONS.down);
      break;
    case WORKSPACE_COMMANDS.zoomIn:
      adjustZoom(state, 0.1);
      setLastAction(`Overview zoom ${Math.round(state.camera.zoom * 100)}%`);
      break;
    case WORKSPACE_COMMANDS.zoomOut:
      adjustZoom(state, -0.1);
      setLastAction(`Overview zoom ${Math.round(state.camera.zoom * 100)}%`);
      break;
    case WORKSPACE_COMMANDS.resetViewport:
      resetViewport(state);
      setLastAction("Overview recentered");
      break;
    case WORKSPACE_COMMANDS.nextContext:
      if (contextCount > 0) {
        switchToContext((state.activeContextIndex + 1) % contextCount);
        return;
      }
      break;
    case WORKSPACE_COMMANDS.previousContext:
      if (contextCount > 0) {
        switchToContext((state.activeContextIndex - 1 + contextCount) % contextCount);
        return;
      }
      break;
    case WORKSPACE_COMMANDS.newContext:
      openContextModal({ mode: "create" });
      return;
    default:
      if (CONTEXT_COMMANDS.includes(command)) {
        switchToContext(Number(command.slice(-1)) - 1);
        return;
      }
      break;
  }

  saveState();
  render();
}

function renderContextButtons() {
  if (!dom.contextList) {
    return;
  }

  dom.contextList.innerHTML = state.contexts
    .map((context, index) => `
      <li class="context-row">
        <button
          class="context-item ${index === state.activeContextIndex ? "active" : ""}"
          data-context-index="${index}"
          type="button"
        >${formatContextLabel(context.label, index)}</button>
        <button
          class="toolbar-button toolbar-button--ghost context-rename"
          data-rename-context-index="${index}"
          type="button"
          aria-label="Edit ${formatContextLabel(context.label, index)}"
        >Edit</button>
      </li>
    `)
    .join("");
}

function renderMinimap(visiblePanels, activePanel) {
  dom.minimap.classList.toggle("is-hidden", visiblePanels.length === 0);
  dom.minimapSize.textContent = `${visiblePanels.length} terminal${visiblePanels.length === 1 ? "" : "s"}`;

  if (visiblePanels.length === 0) {
    dom.minimapGrid.innerHTML = "";
    return;
  }

  const bounds = getBounds(visiblePanels);
  const width = dom.minimapGrid.clientWidth || 228;
  const spanX = Math.max(bounds.width + 1, 1);
  const spanY = Math.max(bounds.height + 1, 1);
  const gutter = 10;
  const cellWidth = Math.max(12, Math.min(18, Math.floor((width - gutter * 2) / spanX)));
  const cellHeight = Math.max(12, Math.min(16, Math.floor(140 / spanY)));
  const gridHeight = Math.max(88, Math.min(176, spanY * cellHeight + gutter * 2));
  dom.minimapGrid.style.height = `${gridHeight}px`;

  dom.minimapGrid.innerHTML = visiblePanels
    .map((panel) => {
      const left = (panel.x - bounds.minX) * cellWidth + gutter;
      const top = (panel.y - bounds.minY) * cellHeight + gutter;
      const active = panel.id === activePanel?.id ? "is-active" : "";

      return `<button class="minimap-node ${active}" data-focus-panel="${panel.id}" style="left:${left}px;top:${top}px;" aria-label="${panel.title}"></button>`;
    })
    .join("");
}

function renderOverview(visiblePanels, activePanel) {
  if (visiblePanels.length === 0) {
    dom.overviewCanvas.innerHTML = "";
    return;
  }

  const bounds = getBounds(visiblePanels);
  const cellWidth = 312;
  const rowHeight = 212;
  const offsetX = 136;
  const offsetY = 88;

  dom.overviewCanvas.innerHTML = `
    <div class="overview-viewport" style="transform: translate(${state.camera.x}px, ${state.camera.y}px) scale(${state.camera.zoom});">
      ${visiblePanels
        .map((panel) => {
          const left = (panel.x - bounds.minX) * cellWidth + offsetX;
          const top = (panel.y - bounds.minY) * rowHeight + offsetY;
          const active = panel.id === activePanel?.id ? "is-active" : "";

          return `
            <article class="overview-node ${active}" data-focus-panel="${panel.id}" style="left:${left}px;top:${top}px;">
              <h3>${panel.title}</h3>
              <p>${PANEL_TYPES[panel.type].summary}</p>
              <div class="overview-node-meta">
                <span>${panel.x}, ${panel.y}</span>
                <span>${panel.cwdLabel || panel.cwd}</span>
              </div>
            </article>
          `;
        })
        .join("")}
    </div>
  `;
}

function renderFocus(activePanel) {
  if (!activePanel) {
    dom.focusTitle.textContent = "No active terminal";
    dom.focusPath.textContent = formatContextLabel(getActiveContextLabel(), state.activeContextIndex);
    dom.focusPosition.textContent = "0, 0";
    return;
  }

  dom.focusTitle.textContent = activePanel.title;
  dom.focusPath.textContent = activePanel.cwdLabel || activePanel.cwd;
  dom.focusPosition.textContent = `${activePanel.x}, ${activePanel.y}`;
}

async function render() {
  ensurePanelSessions();
  renderContextButtons();
  renderContextModal();

  const visiblePanels = getVisiblePanels(state);
  const activePanel = ensureActivePanel(state);
  const workspaceSnapshot = getWorkspaceSnapshot(state);
  updateEngineLabel(activePanel);
  renderFocus(activePanel);
  renderOverview(visiblePanels, activePanel);
  renderMinimap(visiblePanels, activePanel);
  dom.appShell.classList.toggle("app-shell--sidebar-hidden", !state.sidebarVisible);
  dom.sidebar?.setAttribute("aria-hidden", String(!state.sidebarVisible));

  dom.focusShell.classList.toggle("is-hidden", state.mode !== "focus" || visiblePanels.length === 0);
  dom.overviewShell.classList.toggle("is-hidden", state.mode !== "overview" || visiblePanels.length === 0);
  dom.emptyShell.classList.toggle("is-hidden", visiblePanels.length !== 0);
  dom.shortcutsOverlay.classList.toggle("is-hidden", !state.shortcutsVisible);

  if (dom.modeLabel) {
    dom.modeLabel.textContent = state.mode === "focus" ? "Focus" : "Overview";
  }
  if (dom.toolbarContext) {
    dom.toolbarContext.textContent = formatContextLabel(getActiveContextLabel(), state.activeContextIndex);
  }
  if (dom.workspaceStorageLabel) {
    dom.workspaceStorageLabel.textContent = uiState.workspaceStorageSource === "disk" ? "Disk" : "Browser";
  }
  if (dom.workspacePath) {
    dom.workspacePath.textContent = uiState.workspaceStoragePath;
  }
  if (dom.workspaceJsonShell) {
    dom.workspaceJsonShell.classList.toggle("is-hidden", !uiState.workspaceInspectorVisible);
  }
  if (dom.workspaceJson) {
    dom.workspaceJson.value = workspaceSnapshot.json;
  }

  if (state.mode === "focus" && activePanel) {
    await mountActiveTerminal(activePanel);
  }

  dom.stage.dataset.mode = state.mode;
}

function bindUiEvents() {
  document.addEventListener("click", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    if (!target) {
      return;
    }

    const commandButton = target.closest("[data-command]");
    if (commandButton) {
      runCommand(commandButton.getAttribute("data-command"));
      return;
    }

    const focusTarget = target.closest("[data-focus-panel]");
    if (focusTarget) {
      const panelId = focusTarget.getAttribute("data-focus-panel");
      focusPanel(state, panelId);
      state.mode = "focus";
      setLastAction(`Focused ${getActivePanel(state)?.title}`);
      saveState();
      render();
      return;
    }

    const contextButton = target.closest("[data-context-index]");
    if (contextButton) {
      switchToContext(Number(contextButton.getAttribute("data-context-index")));
      return;
    }

    const renameButton = target.closest("[data-rename-context-index]");
    if (renameButton) {
      openContextModal({
        mode: "rename",
        index: Number(renameButton.getAttribute("data-rename-context-index")),
      });
    }
  });

  dom.newContextButton?.addEventListener("click", () => {
    openContextModal({ mode: "create" });
  });

  dom.saveWorkspaceButton?.addEventListener("click", () => {
    runCommand(WORKSPACE_COMMANDS.saveWorkspace);
  });

  dom.toolbarSaveWorkspaceButton?.addEventListener("click", () => {
    runCommand(WORKSPACE_COMMANDS.saveWorkspace);
  });

  dom.toggleWorkspaceJsonButton?.addEventListener("click", toggleWorkspaceJson);
  dom.toolbarToggleJsonButton?.addEventListener("click", toggleWorkspaceJson);

  dom.contextCancelButton?.addEventListener("click", closeContextModal);
  dom.contextCloseButton?.addEventListener("click", closeContextModal);
  dom.contextForm?.addEventListener("submit", (event) => {
    event.preventDefault();
    submitContextModal();
  });

  window.addEventListener("plexi:command", (event) => {
    const command = event.detail?.command;
    if (command) {
      runCommand(command);
    }
  });

  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && uiState.contextModalOpen) {
      event.preventDefault();
      closeContextModal();
      return;
    }

    if (!event.defaultPrevented) {
      handleShortcutKeydown(event);
    }
  });
}

bindUiEvents();
syncViewportMetrics();
window.addEventListener("resize", syncViewportMetrics);
window.visualViewport?.addEventListener("resize", syncViewportMetrics);

window.__PLEXI_DEBUG__ = {
  getState: () => clone(state),
  getTerminalProfile,
  runCommand,
  reset: () => {
    state.panels.forEach((panel) => closePanelSession(panel.id));
    void sessionBridge.reset();
    panelMeta.clear();
    panelBuffers.clear();
    state = bootDefaultState();
    uiState.workspaceInspectorVisible = false;
    closeContextModal();
    saveState();
    render();
  },
};

async function initializeApp() {
  const [info, hydrated] = await Promise.all([
    sessionBridge.getBackendInfo(),
    hydrateWorkspaceState(sessionBridge),
  ]);

  backendInfo = info;
  state = hydrated.state;
  updateWorkspaceStorage(hydrated.storage);

  if (sessionBridge.mode === "live" && !hydrated.storage) {
    updateWorkspaceStorage({
      path: "Workspace file unavailable",
      source: "disk",
    });
  }

  if (sessionBridge.mode === "live" && hydrated.storage && hydrated.state.lastAction === "Workspace ready") {
    saveState();
  }

  syncViewportMetrics();
  updateEngineLabel(getActivePanel(state));
  await render();
}

void initializeApp();
console.log("Plexi terminal workspace loaded");
