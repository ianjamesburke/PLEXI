import {
  DIRECTIONS,
  clone,
  closePanelRecord,
  createContextRecord,
  createPanelRecord,
  createTopLevelPanelRecord,
  deleteContextRecord,
  getActiveNode,
  ensureActivePanel,
  focusDirectionalPanel,
  focusPanel,
  getActivePanel,
  getBounds,
  getNodeForPanelId,
  getNodePaneBounds,
  getVisibleNodes,
  getVisiblePanels,
moveContextRecord,
  toggleContextPinned,
  renameContextRecord,
  setContextIndex,
} from "../shared/workspace-state.js";
import { CONTEXT_COMMANDS, PANE_COMMANDS, WORKSPACE_COMMANDS, isWorkspaceCommand } from "../shared/commands.js";
import { getDisplayContextLabel } from "../shared/workspace-document.js";
import { resolveKeybind } from "../shared/keybinds.js";
import { createSessionBridge } from "./tauri-session-bridge.js";
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
  hydrateWorkspaceState,
  loadWorkspaceState,
  saveWorkspaceState,
} from "./workspace-storage.js";
import {
  adjustTerminalFontSize,
  createTerminalRuntime,
  ensureTerminalFont,
  ensureXtermAssets,
  getTerminalProfile,
  getTerminalZoomStep,
  setXtermError,
} from "./xterm-runtime.js";
import { loadConfig, getConfigWarnings } from "./plexi-config.js";

const paneRuntimes = new Map();
const panelBuffers = new Map();
const panelMeta = new Map();
const panelSessions = new Set();
const panelSessionFailed = new Set();
const outputSequences = window.__PLEXI_OUTPUT_SEQUENCES__ || (window.__PLEXI_OUTPUT_SEQUENCES__ = new Map());
let backendInfo = null;
let homeDirectory = null;
let state = bootDefaultState();
const uiState = {
  workspaceStorageSource: "browser",
  contextModalOpen: false,
  overviewOpen: false,
  contextRenameIndex: null,
  contextDeleteConfirming: false,
  contextDeleteTimer: null,
  toastTimer: null,
};

applyPlatformClasses();

const sessionBridge = createSessionBridge({
  onStarted(message) {
    outputSequences.delete(message.panelId);
    panelMeta.set(message.panelId, {
      shellName: message.shellName,
      shellPath: message.shellPath,
      backend: message.backend,
    });
    const panel = state.panels.find((item) => item.id === message.panelId);
    if (!panel) {
      return;
    }

    if (message.homeDir) {
      homeDirectory = homeDirectory || message.homeDir;
    }
    if (message.cwd) {
      panel.cwd = message.cwd;
      panel.cwdLabel = formatPathLabel(message.cwd, homeDirectory);
      homeDirectory = homeDirectory || inferHomeDirectory(panel.cwd, panel.cwdLabel);
    }
    saveState();
    render();
  },
  onOutput(message) {
    if (typeof message.seq === "number" && message.seq > 0) {
      const lastSeq = outputSequences.get(message.panelId) ?? 0;
      if (message.seq <= lastSeq) {
        return;
      }
      outputSequences.set(message.panelId, message.seq);
    }

    const { cleaned, cwd } = extractSessionOutputMetadata(message.data);

    if (cwd) {
      updatePanelDirectory(message.panelId, cwd);
    }

    if (!cleaned) {
      return;
    }

    appendPanelBuffer(message.panelId, cleaned);

    const runtime = getPaneRuntime(message.panelId);
    if (runtime) {
      runtime.enqueueWrite(cleaned, { scrollToBottom: !runtime.interactive });
    }
  },
  onExit(message) {
    panelSessions.delete(message.panelId);
    outputSequences.delete(message.panelId);
    appendPanelBuffer(message.panelId, `\r\n[session exited ${message.exitCode}]\r\n`);

    const runtime = getPaneRuntime(message.panelId);
    if (runtime) {
      runtime.enqueueWrite(`\r\n[session exited ${message.exitCode}]\r\n`, { scrollToBottom: true });
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
    const runtime = getPaneRuntime(panelId);
    if (runtime) {
      runtime.terminal.clear();
    }
  },
});

if (sessionBridge.mode !== "live" && sessionBridge.mode !== "tauri") {
  state = loadWorkspaceState();
}

let saveScheduled = false;
function saveState() {
  if (saveScheduled) return;
  saveScheduled = true;
  const schedule = window.requestIdleCallback || ((cb) => setTimeout(cb, 0));
  schedule(() => {
    saveScheduled = false;
    const snapshot = clone(state);
    void saveWorkspaceState(snapshot, sessionBridge)
      .then(updateWorkspaceStorage)
      .catch(() => {
        showToast("Workspace save failed");
      });
  });
}

function showToast(message) {
  if (!dom.toastLayer || !message) {
    return;
  }

  const toast = document.createElement("div");
  toast.className = "toast";
  toast.textContent = message;
  dom.toastLayer.replaceChildren(toast);
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

  const visibleNodes = getVisibleNodes(state);
  const activeNode = getActiveNode(state);
  renderMinimap(visibleNodes, activeNode);
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

function clearNode(node) {
  node?.replaceChildren();
}

function createRenameIcon() {
  const namespace = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(namespace, "svg");
  svg.setAttribute("width", "12");
  svg.setAttribute("height", "12");
  svg.setAttribute("viewBox", "0 0 12 12");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.5");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");

  const path = document.createElementNS(namespace, "path");
  path.setAttribute("d", "M7 2l3 3-7 7H0V9z");
  svg.append(path);

  return svg;
}

function resetDeleteConfirmation() {
  uiState.contextDeleteConfirming = false;

  if (uiState.contextDeleteTimer) {
    window.clearTimeout(uiState.contextDeleteTimer);
    uiState.contextDeleteTimer = null;
  }
}

function replayBuffer(runtime) {
  runtime.mountNode?.classList.add("terminal-mount--hydrating");
  runtime.terminal.options.cursorBlink = false;
  runtime.pendingWrites.length = 0;
  runtime.needsScrollToBottom = false;
  if (runtime.writeFrame) {
    window.cancelAnimationFrame(runtime.writeFrame);
    runtime.writeFrame = 0;
  }
  if (runtime.resizeFrame) {
    window.cancelAnimationFrame(runtime.resizeFrame);
    runtime.resizeFrame = 0;
  }
  runtime.terminal.reset();
  runtime.terminal.clear();
  // Fit the terminal and notify the PTY BEFORE replaying the buffer so the PTY
  // is already at the correct dimensions. This avoids a post-replay SIGWINCH
  // causing the shell to redraw its prompt on top of the replayed one.
  try {
    runtime.fitAddon.fit();
    if (runtime.terminal.cols > 0 && runtime.terminal.rows > 0) {
      void sessionBridge.resizeSession({
        panelId: runtime.panel.id,
        cols: runtime.terminal.cols,
        rows: runtime.terminal.rows,
      });
    }
  } catch (_error) {
    // Ignore fit errors during replay — container may not be fully laid out yet.
  }
  runtime.terminal.write(panelBuffers.get(runtime.panel.id) || "", () => {
    runtime.terminal.scrollToBottom?.();
    runtime.mountNode?.classList.remove("terminal-mount--hydrating");
    runtime.terminal.options.cursorBlink = runtime.interactive;
    if (runtime.interactive) {
      window.requestAnimationFrame(() => {
        runtime.terminal.focus();
      });
    }
  });
}

function getPaneRuntime(panelId) {
  return paneRuntimes.get(panelId) || null;
}

function getActiveRuntime() {
  return getPaneRuntime(state.activePanelId);
}

function disposePaneRuntime(panelId) {
  const runtime = getPaneRuntime(panelId);
  if (!runtime) {
    return;
  }

  runtime.dispose();
  paneRuntimes.delete(panelId);
}

function disposeAllPaneRuntimes() {
  for (const panelId of paneRuntimes.keys()) {
    disposePaneRuntime(panelId);
  }
}

function updatePanelDirectory(panelId, cwd) {
  const panel = state.panels.find((item) => item.id === panelId);
  if (!panel || panel.cwd === cwd) {
    return;
  }

  panel.cwd = cwd;
  panel.cwdLabel = formatPathLabel(cwd, homeDirectory);
  saveState();

  if (getPaneRuntime(panelId)) {
    render();
  }
}

async function copySelection(runtime) {
  const selection = runtime?.terminal?.getSelection?.();

  if (!selection) {
    return;
  }

  try {
    await sessionBridge.writeClipboardText(selection);
    setLastAction("Selection copied");
  } catch (_error) {
    setLastAction("Clipboard copy failed");
  }

  saveState();
  render();
}

async function pasteClipboard(runtime) {
  try {
    const text = await sessionBridge.readClipboardText();

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
  if (!panel || panel.type !== "terminal" || panelSessions.has(panel.id) || panelSessionFailed.has(panel.id)) {
    return;
  }

  panelSessions.add(panel.id);
  clearPanelBuffer(panel.id);

  try {
    await sessionBridge.openSession({
      panelId: panel.id,
      cwd: panel.cwd,
      cols: getPaneRuntime(panel.id)?.terminal?.cols || getActiveRuntime()?.terminal?.cols || 80,
      rows: getPaneRuntime(panel.id)?.terminal?.rows || getActiveRuntime()?.terminal?.rows || 24,
    });
  } catch (error) {
    panelSessions.delete(panel.id);
    panelSessionFailed.add(panel.id);
    const message = error?.message || String(error);
    appendPanelBuffer(panel.id, `\r\n[session failed] ${message}\r\n`);
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
  disposePaneRuntime(panelId);
  panelSessions.delete(panelId);
  panelSessionFailed.delete(panelId);
  panelMeta.delete(panelId);
  panelBuffers.delete(panelId);
  outputSequences.delete(panelId);
  void sessionBridge.closeSession({ panelId });
}

function createPaneRuntime(panel, mountNode, interactive) {
  return createTerminalRuntime({
    panel,
    mountNode,
    interactive,
    onData(runtime, rawData) {
      runtime.panel.hasReceivedInput = true;
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
        // Let the browser handle Cmd+V natively — xterm.js picks up the
        // paste event via its own listener and routes it through onData.
        // Intercepting it ourselves and calling navigator.clipboard.readText()
        // triggers a WebView permission popup on macOS.
        return true;
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
    onLinkClick(uri) {
      if (uri) {
        void sessionBridge.openExternalUrl(uri);
      }
    },
    replayBuffer,
  });
}

async function syncVisiblePaneRuntimes(activeNode, activePanel) {
  if (!activeNode || !activePanel) {
    disposeAllPaneRuntimes();
    return;
  }

  try {
    await ensureXtermAssets();
    await ensureTerminalFont();
  } catch (error) {
    setXtermError();
    disposeAllPaneRuntimes();
    const errorMount = dom.focusNodeGrid?.querySelector("[data-panel-terminal-mount]");
    if (errorMount instanceof HTMLElement) {
      errorMount.textContent = String(error);
      errorMount.classList.add("terminal-mount--error");
    }
    return;
  }

  const desiredPanels = activeNode.panes.filter((panel) => panel.type === "terminal");
  const desiredIds = new Set(desiredPanels.map((panel) => panel.id));

  for (const panelId of paneRuntimes.keys()) {
    if (!desiredIds.has(panelId)) {
      disposePaneRuntime(panelId);
    }
  }

  const mounts = new Map(
    [...document.querySelectorAll("[data-panel-terminal-mount]")]
      .map((node) => [node.getAttribute("data-panel-terminal-mount"), node])
      .filter(([panelId, node]) => panelId && node instanceof HTMLElement),
  );

  desiredPanels.forEach((panel) => {
    const mountNode = mounts.get(panel.id);
    if (!(mountNode instanceof HTMLElement)) {
      disposePaneRuntime(panel.id);
      return;
    }

    mountNode.classList.remove("terminal-mount--loading", "terminal-mount--error");
    const interactive = panel.id === activePanel.id;
    const existing = getPaneRuntime(panel.id);
    const needsRecreate = !existing
      || existing.panel.id !== panel.id
      || existing.panel.fontSize !== panel.fontSize
      || existing.terminal.element?.parentElement !== mountNode;

    if (needsRecreate) {
      disposePaneRuntime(panel.id);
      paneRuntimes.set(panel.id, createPaneRuntime(panel, mountNode, interactive));
      return;
    }

    existing.panel = panel;
    existing.mountNode = mountNode;
    existing.interactive = interactive;
    mountNode.classList.toggle("terminal-mount--preview", !interactive);
    mountNode.classList.toggle("terminal-mount--active", interactive);
    existing.terminal.options.disableStdin = !interactive;
    existing.terminal.options.cursorBlink = interactive;
    existing.resizeHandler();
    if (interactive) {
      existing.terminal.focus();
    }
  });
}


function renderContextModal() {
  if (!dom.contextModal) {
    return;
  }

  dom.contextModal.classList.toggle("is-hidden", !uiState.contextModalOpen);

  const titleEl = dom.contextModal.querySelector("#context-modal-title");
  if (titleEl) {
    titleEl.textContent = uiState.contextRenameIndex !== null ? "Edit context" : "New context";
  }

  const deleteBtn = dom.contextDeleteButton;
  if (deleteBtn) {
    deleteBtn.classList.toggle("is-hidden", uiState.contextRenameIndex === null);
    deleteBtn.textContent = uiState.contextDeleteConfirming ? "Are you sure?" : "Delete";
  }

  const context = uiState.contextRenameIndex !== null ? state.contexts[uiState.contextRenameIndex] : null;
  dom.contextPinButton?.classList.toggle("is-hidden", !context);
  dom.contextMoveUpButton?.classList.toggle("is-hidden", !context);
  dom.contextMoveDownButton?.classList.toggle("is-hidden", !context);
  if (dom.contextPinButton && context) {
    dom.contextPinButton.textContent = context.pinned ? "Unpin" : "Pin";
  }
}

function openContextModal() {
  resetDeleteConfirmation();
  uiState.contextModalOpen = true;
  uiState.contextRenameIndex = null;
  renderContextModal();

  if (dom.contextNameInput) {
    dom.contextNameInput.value = `Context ${getContextCount() + 1}`;
  }

  window.requestAnimationFrame(() => {
    dom.contextNameInput?.focus();
    dom.contextNameInput?.select();
  });
}

function renameContext(index) {
  if (!Number.isInteger(index) || index < 0 || index >= state.contexts.length) {
    return;
  }

  resetDeleteConfirmation();
  uiState.contextModalOpen = true;
  uiState.contextRenameIndex = index;
  renderContextModal();

  if (dom.contextNameInput) {
    dom.contextNameInput.value = state.contexts[index]?.label || "";
  }

  window.requestAnimationFrame(() => {
    dom.contextNameInput?.focus();
    dom.contextNameInput?.select();
  });
}

function closeContextModal() {
  resetDeleteConfirmation();
  uiState.contextModalOpen = false;
  uiState.contextRenameIndex = null;
  renderContextModal();
}

function deleteContext() {
  const index = uiState.contextRenameIndex;
  if (index === null || index < 0 || index >= state.contexts.length) {
    setLastAction("Cannot delete: invalid context index");
    return;
  }

  if (state.contexts.length <= 1) {
    setLastAction("Cannot delete the last context");
    return;
  }

  const contextLabel = state.contexts[index]?.label || `Context ${index + 1}`;

  if (!uiState.contextDeleteConfirming) {
    uiState.contextDeleteConfirming = true;
    renderContextModal();
    uiState.contextDeleteTimer = window.setTimeout(() => {
      resetDeleteConfirmation();
      renderContextModal();
    }, 3000);
    return;
  }

  resetDeleteConfirmation();

  // Close sessions for panels in this context before removing them
  state.panels
    .filter((panel) => panel.contextIndex === index)
    .forEach((panel) => closePanelSession(panel.id));

  deleteContextRecord(state, index);
  setLastAction(`Context ${contextLabel} deleted`);
  closeContextModal();
  saveState();
  render();
}

function submitContextModal() {
  const label = dom.contextNameInput?.value?.trim();

  if (!label) {
    dom.contextNameInput?.focus();
    return;
  }

  if (uiState.contextRenameIndex !== null) {
    renameContextRecord(state, uiState.contextRenameIndex, label);
    setLastAction(`Context renamed to ${label}`);
  } else {
    createContextRecord(state, label);
    const panel = createTopLevelPanelRecord(state, { direction: DIRECTIONS.right });
    if (panel) {
      clearPanelBuffer(panel.id);
      void ensurePanelSession(panel);
    }
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
  if (index === state.activeContextIndex) {
    return false;
  }
  setContextIndex(state, index);
  setLastAction(`Context ${formatContextLabel(getActiveContextLabel())}`);
  saveState();
  render();
  return true;
}

function closeOverview() {
  uiState.overviewOpen = false;
}

function toggleOverview() {
  uiState.overviewOpen = !uiState.overviewOpen;
  setLastAction(uiState.overviewOpen ? "Workspace overview open" : "Workspace overview closed");
}

function focusVisiblePane(index) {
  const visiblePanels = getVisiblePanels(state);
  const panel = visiblePanels[index];

  if (!panel) {
    setLastAction(`Pane ${index + 1} is not available`);
    return false;
  }

  focusPanel(state, panel.id);
  setLastAction(`Focused ${panel.title}`);
  saveState();
  render();
  return true;
}

function createTerminal(direction, cwd = null, cwdLabel = null) {
  if (state.contexts.length === 0) {
    createContextRecord(state, "");
  }

  const panel = createPanelRecord(state, { direction, cwd, cwdLabel });
  if (!panel) {
    setLastAction("Split group is full");
    return;
  }
  clearPanelBuffer(panel.id);
  void ensurePanelSession(panel);
  setLastAction(
    direction === DIRECTIONS.down
      ? `${panel.title} split below`
      : `${panel.title} split right`,
  );
}

function createTopLevelTerminal(direction, cwd = null, cwdLabel = null) {
  if (state.contexts.length === 0) {
    createContextRecord(state, "");
  }

  const panel = createTopLevelPanelRecord(state, { direction, cwd, cwdLabel });
  if (!panel) {
    setLastAction("Unable to create top-level node");
    return;
  }

  clearPanelBuffer(panel.id);
  void ensurePanelSession(panel);
  setLastAction(
    direction === DIRECTIONS.down
      ? `${panel.title} opened in a node below`
      : `${panel.title} opened in a node to the right`,
  );
}

function closeActiveTerminal() {
  const activePanel = getActivePanel(state);

  if (!activePanel) {
    setLastAction("No terminal to close");
    return;
  }

  const removed = closePanelRecord(state, activePanel.id);

  if (removed) {
    closePanelSession(removed.id);
    setLastAction(`${removed.title} closed`);
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
      pane_1: WORKSPACE_COMMANDS.pane1,
      pane_2: WORKSPACE_COMMANDS.pane2,
      pane_3: WORKSPACE_COMMANDS.pane3,
      pane_4: WORKSPACE_COMMANDS.pane4,
      pane_5: WORKSPACE_COMMANDS.pane5,
      pane_6: WORKSPACE_COMMANDS.pane6,
      pane_7: WORKSPACE_COMMANDS.pane7,
      pane_8: WORKSPACE_COMMANDS.pane8,
      pane_9: WORKSPACE_COMMANDS.pane9,
      context_1: WORKSPACE_COMMANDS.context1,
      context_2: WORKSPACE_COMMANDS.context2,
      context_3: WORKSPACE_COMMANDS.context3,
      context_4: WORKSPACE_COMMANDS.context4,
      context_5: WORKSPACE_COMMANDS.context5,
      context_6: WORKSPACE_COMMANDS.context6,
      context_7: WORKSPACE_COMMANDS.context7,
      context_8: WORKSPACE_COMMANDS.context8,
      context_9: WORKSPACE_COMMANDS.context9,
      next_context: WORKSPACE_COMMANDS.nextContext,
      previous_context: WORKSPACE_COMMANDS.previousContext,
      focus_down: WORKSPACE_COMMANDS.focusDown,
      focus_left: WORKSPACE_COMMANDS.focusLeft,
      focus_right: WORKSPACE_COMMANDS.focusRight,
      focus_up: WORKSPACE_COMMANDS.focusUp,
new_node_down: WORKSPACE_COMMANDS.newNodeDown,
      new_node_right: WORKSPACE_COMMANDS.newNodeRight,
      new_terminal_down: WORKSPACE_COMMANDS.newTerminalDown,
      new_terminal_right: WORKSPACE_COMMANDS.newTerminalRight,
      quit_application: "quit-application",
      save_workspace: WORKSPACE_COMMANDS.saveWorkspace,
      toggle_minimap: WORKSPACE_COMMANDS.toggleMinimap,
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

  return false;
}

function runCommand(command) {
  if (command === "quit-application") {
    void sessionBridge.quitApplication?.();
    return;
  }

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
    case WORKSPACE_COMMANDS.newNodeRight:
      createTopLevelTerminal(DIRECTIONS.right);
      break;
    case WORKSPACE_COMMANDS.newNodeDown:
      createTopLevelTerminal(DIRECTIONS.down);
      break;
    case WORKSPACE_COMMANDS.closeTerminal:
      closeActiveTerminal();
      break;
case WORKSPACE_COMMANDS.toggleSidebar:
      state.sidebarVisible = !state.sidebarVisible;
      setLastAction(state.sidebarVisible ? "Sidebar shown" : "Sidebar hidden");
      break;
    case WORKSPACE_COMMANDS.toggleMinimap:
      toggleOverview();
      break;
    case WORKSPACE_COMMANDS.toggleShortcuts:
    case WORKSPACE_COMMANDS.showShortcuts:
      toggleShortcuts();
      break;
    case WORKSPACE_COMMANDS.saveWorkspace:
      saveState();
      setLastAction("Workspace saved");
      break;
    case WORKSPACE_COMMANDS.zoomIn:
      if (state.activePanelId) {
        const activeRuntime = getActiveRuntime();
        const activePanel = getActivePanel(state);
        activePanel.fontSize = adjustTerminalFontSize(
          getTerminalZoomStep(),
          activeRuntime,
          activePanel.fontSize,
        );
      }
      setLastAction("Font size increased");
      saveState();
      break;
    case WORKSPACE_COMMANDS.zoomOut:
      if (state.activePanelId) {
        const activeRuntime = getActiveRuntime();
        const activePanel = getActivePanel(state);
        activePanel.fontSize = adjustTerminalFontSize(
          -getTerminalZoomStep(),
          activeRuntime,
          activePanel.fontSize,
        );
      }
      setLastAction("Font size decreased");
      saveState();
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
      openContextModal();
      return;
    default:
      if (PANE_COMMANDS.includes(command)) {
        focusVisiblePane(Number(command.slice(-1)) - 1);
        return;
      }
      if (CONTEXT_COMMANDS.includes(command)) {
        switchToContext(Number(command.slice(-1)) - 1);
        return;
      }
      break;
  }

  saveState();
  render();
}

function getRowFocusMap(contextIndex = state.activeContextIndex) {
  if (!state.rowFocusPanelIdsByContext) {
    state.rowFocusPanelIdsByContext = {};
  }

  if (!state.rowFocusPanelIdsByContext[contextIndex]) {
    state.rowFocusPanelIdsByContext[contextIndex] = {};
  }

  return state.rowFocusPanelIdsByContext[contextIndex];
}

function isRowFocusPanel(panel) {
  if (!panel) return false;
  const rowFocusMap = getRowFocusMap(panel.contextIndex);
  return rowFocusMap[panel.y] === panel.id;
}

function renderContextButtons() {
  if (!dom.contextList) {
    return;
  }

  const rows = state.contexts.map((context, index) => {
    const label = formatContextLabel(context.label, index);
    const row = document.createElement("li");
    row.className = "context-row";

    const contextButton = document.createElement("button");
    contextButton.className = `context-item ${index === state.activeContextIndex ? "active" : ""}`.trim();
    contextButton.dataset.contextIndex = String(index);
    contextButton.type = "button";
    contextButton.textContent = context.pinned ? `★ ${label}` : label;

    const renameButton = document.createElement("button");
    renameButton.className = `toolbar-button toolbar-button--ghost context-rename ${index === state.activeContextIndex ? "context-rename--active" : ""}`.trim();
    renameButton.dataset.renameContextIndex = String(index);
    renameButton.type = "button";
    renameButton.setAttribute("aria-label", `Rename ${label}`);
    renameButton.title = `Rename ${label}`;
    renameButton.append(createRenameIcon());

    row.append(contextButton, renameButton);
    return row;
  });

  dom.contextList.replaceChildren(...rows);
}

function createMinimapNode(nodeRecord, activeNode, options) {
  const node = document.createElement("button");
  const active = nodeRecord.id === activeNode?.id ? "is-active" : "";
  const rowFocus = isRowFocusPanel(nodeRecord.panes.find((pane) => pane.id === nodeRecord.activePaneId) || nodeRecord.panes[0]) ? "is-row-focus" : "";
  const variant = options.variant === "overview" ? "minimap-node--overview" : "minimap-node--sidebar";
  const activePane = nodeRecord.panes.find((pane) => pane.id === nodeRecord.activePaneId) || nodeRecord.panes[0];
  const paneBounds = getNodePaneBounds(nodeRecord);

  node.className = `minimap-node ${variant} ${active} ${rowFocus}`.trim();
  node.type = "button";
  node.dataset.focusPanel = nodeRecord.activePaneId || nodeRecord.panes[0]?.id || "";
  if (Number.isInteger(options.contextIndex)) {
    node.dataset.focusContextIndex = String(options.contextIndex);
  }
  node.style.left = `${options.left}px`;
  node.style.top = `${options.top}px`;
  node.style.width = `${options.width}px`;
  node.style.height = `${options.height}px`;
  node.setAttribute("aria-label", activePane?.title || "Workspace node");

  const surface = document.createElement("div");
  surface.className = "minimap-node__surface";
  nodeRecord.panes.forEach((pane) => {
    const preview = document.createElement("div");
    preview.className = `minimap-pane-preview ${pane.id === nodeRecord.activePaneId ? "is-active" : ""}`.trim();
    preview.style.left = `${((pane.splitX - paneBounds.minX) / paneBounds.width) * 100}%`;
    preview.style.top = `${((pane.splitY - paneBounds.minY) / paneBounds.height) * 100}%`;
    preview.style.width = `${(pane.splitWidth / paneBounds.width) * 100}%`;
    preview.style.height = `${(pane.splitHeight / paneBounds.height) * 100}%`;
    surface.append(preview);
  });
  node.append(surface);

  return node;
}

function buildMinimapNodes(visibleNodes, activeNode, gridElement, options = {}) {
  const bounds = getBounds(visibleNodes);
  const width = gridElement.clientWidth || options.defaultWidth || 228;
  const spanX = Math.max(bounds.width, 1);
  const spanY = Math.max(bounds.height, 1);
  const gutter = options.gutter || 10;
  const nodeGap = options.nodeGap || 4;
  const minNodeSize = options.minNodeSize || 10;
  const maxNodeSize = options.maxNodeSize || options.baseNodeSize || 18;
  const availableWidth = Math.max(width - gutter * 2, minNodeSize);
  const fittedNodeSize = Math.floor((availableWidth - Math.max(0, spanX - 1) * nodeGap) / spanX);
  const nodeSize = Math.max(minNodeSize, Math.min(maxNodeSize, fittedNodeSize));
  const contentHeight = spanY * nodeSize + Math.max(0, spanY - 1) * nodeGap;
  const startX = gutter;
  const startY = gutter;
  const gridHeight = Math.max(options.minHeight || 88, contentHeight + gutter * 2);
  gridElement.style.height = `${gridHeight}px`;

  return visibleNodes.map((nodeRecord) => createMinimapNode(nodeRecord, activeNode, {
    variant: options.variant || "sidebar",
    contextIndex: options.contextIndex,
    left: startX + (nodeRecord.x - bounds.minX) * (nodeSize + nodeGap),
    top: startY + (nodeRecord.y - bounds.minY) * (nodeSize + nodeGap),
    width: nodeSize,
    height: nodeSize,
  }));
}

function renderOverview() {
  if (!dom.overviewShell || !dom.overviewGrid) {
    return;
  }

  dom.overviewShell.classList.toggle("is-hidden", !uiState.overviewOpen);
  dom.overviewShell.setAttribute("aria-hidden", String(!uiState.overviewOpen));
  if (!uiState.overviewOpen) {
    clearNode(dom.overviewGrid);
    return;
  }

  if (dom.overviewSummary) {
    const contextCount = state.contexts.length;
    const nodeCount = state.contexts.reduce((count, _context, contextIndex) => {
      return count + getVisibleNodes({ ...state, activeContextIndex: contextIndex }).length;
    }, 0);
    dom.overviewSummary.textContent = `${contextCount} context${contextCount === 1 ? "" : "s"} · ${nodeCount} node${nodeCount === 1 ? "" : "s"}`;
  }

  const contextSections = state.contexts.map((context, contextIndex) => {
    const section = document.createElement("section");
    section.className = `overview-context ${contextIndex === state.activeContextIndex ? "is-active" : ""}`.trim();

    const visibleNodes = getVisibleNodes({ ...state, activeContextIndex: contextIndex });
    const activePanelId = state.activePanelIdsByContext?.[contextIndex] || null;
    const activeNode = visibleNodes.find((node) => node.panes.some((pane) => pane.id === activePanelId))
      || visibleNodes[0]
      || null;

    const label = document.createElement("div");
    label.className = "overview-context__label";
    const title = document.createElement("span");
    title.textContent = formatContextLabel(context.label, contextIndex);
    const meta = document.createElement("span");
    meta.className = "overview-context__meta";
    const paneCount = visibleNodes.reduce((count, node) => count + node.panes.length, 0);
    meta.textContent = `${visibleNodes.length} node${visibleNodes.length === 1 ? "" : "s"} · ${paneCount} pane${paneCount === 1 ? "" : "s"}`;
    label.append(title, meta);

    const grid = document.createElement("div");
    grid.className = "overview-context__grid";

    if (visibleNodes.length > 0) {
      const nodes = buildMinimapNodes(visibleNodes, activeNode, grid, {
        variant: "overview",
        contextIndex,
        baseNodeSize: 36,
        defaultWidth: grid.clientWidth || dom.overviewShell?.clientWidth || 900,
        minHeight: 88,
        minNodeSize: 20,
        gutter: 12,
        nodeGap: 6,
      });
      grid.replaceChildren(...nodes);
    }

    section.append(label, grid);
    return section;
  });

  dom.overviewGrid.replaceChildren(...contextSections);
}

function renderMinimap(visibleNodes, activeNode) {
  const hasVisibleNodes = visibleNodes.length > 0;
  dom.minimapSize.textContent = `${visibleNodes.length} node${visibleNodes.length === 1 ? "" : "s"}`;

  if (!hasVisibleNodes) {
    clearNode(dom.minimapGrid);
    renderOverview();
    return;
  }

  const sidebarNodes = buildMinimapNodes(visibleNodes, activeNode, dom.minimapGrid, {
    variant: "sidebar",
    baseNodeSize: 16,
    defaultWidth: 228,
    minHeight: 0,
    minNodeSize: 12,
    nodeGap: 4,
  });
  dom.minimapGrid.replaceChildren(...sidebarNodes);
  renderOverview();
}

function renderSparseFocusHints(visibleNodes, activeNode, activePanel) {
  void visibleNodes;
  void activeNode;
  void activePanel;
  dom.focusRightSlot?.classList.add("is-hidden");
  dom.focusBottomSlot?.classList.add("is-hidden");
}

function renderActiveNode(activeNode, activePanel) {
  if (!dom.focusNodeGrid) {
    return;
  }

  if (!activeNode || !activePanel) {
    clearNode(dom.focusNodeGrid);
    return;
  }

  const paneBounds = getNodePaneBounds(activeNode);
  dom.focusNodeGrid.style.gridTemplateColumns = `repeat(${Math.max(4, paneBounds.width)}, minmax(0, 1fr))`;
  dom.focusNodeGrid.style.gridTemplateRows = `repeat(${Math.max(4, paneBounds.height)}, minmax(0, 1fr))`;
  const existingFrames = new Map(
    [...dom.focusNodeGrid.querySelectorAll(".terminal-frame--split")]
      .map((frame) => [frame.getAttribute("data-focus-panel"), frame])
      .filter(([panelId, frame]) => panelId && frame instanceof HTMLElement),
  );
  const nextPanelIds = new Set(activeNode.panes.map((panel) => panel.id));

  activeNode.panes.forEach((panel) => {
    const frame = existingFrames.get(panel.id) || document.createElement("div");
    frame.className = `terminal-frame terminal-frame--split ${panel.id === activePanel.id ? "terminal-frame--active" : "terminal-frame--inactive"}`.trim();
    frame.dataset.focusPanel = panel.id;
    frame.tabIndex = 0;
    frame.style.gridColumn = `${panel.splitX + 1} / span ${panel.splitWidth || 1}`;
    frame.style.gridRow = `${panel.splitY + 1} / span ${panel.splitHeight || 1}`;

    let body = frame.querySelector(".pane-body");
    if (!(body instanceof HTMLElement)) {
      body = document.createElement("div");
      body.className = "pane-body";
      frame.append(body);
    }

    let mount = body.querySelector(".terminal-mount");
    if (!(mount instanceof HTMLElement)) {
      mount = document.createElement("div");
      body.append(mount);
    }

    mount.className = `terminal-mount ${panel.id === activePanel.id ? "terminal-mount--active" : "terminal-mount--preview"}`.trim();
    mount.dataset.panelTerminalMount = panel.id;
    if (panel.id === activePanel.id) {
      mount.id = "terminal-mount";
    } else {
      mount.removeAttribute("id");
    }
    mount.setAttribute("aria-label", panel.id === activePanel.id ? "Active terminal" : `${panel.title} preview`);

    dom.focusNodeGrid.append(frame);
  });

  existingFrames.forEach((frame, panelId) => {
    if (!nextPanelIds.has(panelId)) {
      frame.remove();
    }
  });
}

function renderFocus(activePanel) {
  if (uiState.overviewOpen) {
    if (dom.toolbarContext) {
      dom.toolbarContext.textContent = "Workspace Overview";
    }
    dom.focusPath.textContent = "";
    if (dom.focusProcess) {
      dom.focusProcess.textContent = "";
    }
    return;
  }

  const activeContext = state.contexts[state.activeContextIndex];
  const contextLabel = activeContext
    ? (activeContext.label || `Context ${state.activeContextIndex + 1}`)
    : `Context 1`;

  if (dom.toolbarContext) {
    dom.toolbarContext.textContent = contextLabel;
  }

  if (!activePanel) {
    dom.focusPath.textContent = "";
    if (dom.focusProcess) {
      dom.focusProcess.textContent = "";
    }
    return;
  }

  dom.focusPath.textContent = activePanel.cwdLabel || activePanel.cwd || "~";
  if (dom.focusProcess) {
    const meta = panelMeta.get(activePanel.id);
    dom.focusProcess.textContent = meta?.shellName || "";
  }
}

function renderToolbarState(activePanel) {
  const hasActivePanel = Boolean(activePanel) && !uiState.overviewOpen;
  dom.focusPath?.classList.toggle("is-hidden", !hasActivePanel);
  dom.focusProcess?.classList.toggle("is-hidden", !hasActivePanel);
}

async function render() {
  ensurePanelSessions();
  renderContextButtons();
  renderContextModal();

  const visibleNodes = getVisibleNodes(state);
  const activePanel = ensureActivePanel(state);
  const activeNode = getActiveNode(state);
  renderFocus(activePanel);
  renderToolbarState(activePanel);
  renderMinimap(visibleNodes, activeNode);
  renderSparseFocusHints(visibleNodes, activeNode, activePanel);
  dom.appShell.classList.toggle("app-shell--sidebar-hidden", !state.sidebarVisible);
  dom.sidebar?.setAttribute("aria-hidden", String(!state.sidebarVisible));

  dom.focusShell.classList.toggle("is-hidden", uiState.overviewOpen || visibleNodes.length === 0);
  dom.emptyShell.classList.toggle("is-hidden", uiState.overviewOpen || visibleNodes.length !== 0);
  dom.shortcutsOverlay.classList.toggle("is-hidden", uiState.overviewOpen || !state.shortcutsVisible);

  renderActiveNode(activeNode, activePanel);
  await syncVisiblePaneRuntimes(activeNode, activePanel);
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
      const contextIndexValue = focusTarget.getAttribute("data-focus-context-index");
      if (contextIndexValue !== null) {
        const contextIndex = Number(contextIndexValue);
        if (Number.isInteger(contextIndex) && contextIndex !== state.activeContextIndex) {
          setContextIndex(state, contextIndex);
        }
      }
      const panelId = focusTarget.getAttribute("data-focus-panel");
      focusPanel(state, panelId);
      closeOverview();
      setLastAction(`Focused ${getActivePanel(state)?.title}`);
      saveState();
      render();
      return;
    }

    const renameButton = target.closest("[data-rename-context-index]");
    if (renameButton) {
      renameContext(Number(renameButton.getAttribute("data-rename-context-index")));
      return;
    }

    const contextButton = target.closest("[data-context-index]");
    if (contextButton) {
      switchToContext(Number(contextButton.getAttribute("data-context-index")));
    }
  });

  dom.newContextButton?.addEventListener("click", () => {
    openContextModal();
  });

  dom.contextCancelButton?.addEventListener("click", closeContextModal);
  
  dom.contextDeleteButton?.addEventListener("click", (event) => {
    event.preventDefault();
    deleteContext();
  });

  dom.contextPinButton?.addEventListener("click", (event) => {
    event.preventDefault();
    const index = uiState.contextRenameIndex;
    if (index === null) {
      return;
    }
    const pinned = toggleContextPinned(state, index);
    setLastAction(pinned ? "Context pinned" : "Context unpinned");
    saveState();
    renderContextModal();
    render();
  });

  dom.contextMoveUpButton?.addEventListener("click", (event) => {
    event.preventDefault();
    const index = uiState.contextRenameIndex;
    if (index === null) {
      return;
    }
    if (!moveContextRecord(state, index, -1)) {
      setLastAction("Context cannot move up");
      return;
    }
    uiState.contextRenameIndex -= 1;
    setLastAction("Context moved up");
    saveState();
    renderContextModal();
    render();
  });

  dom.contextMoveDownButton?.addEventListener("click", (event) => {
    event.preventDefault();
    const index = uiState.contextRenameIndex;
    if (index === null) {
      return;
    }
    if (!moveContextRecord(state, index, 1)) {
      setLastAction("Context cannot move down");
      return;
    }
    uiState.contextRenameIndex += 1;
    setLastAction("Context moved down");
    saveState();
    renderContextModal();
    render();
  });
  
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

  const tauriWindow = window.__TAURI__?.window ?? window.__TAURI__?.webviewWindow;
  if (tauriWindow) {
    const dragRegions = document.querySelectorAll('[data-tauri-drag-region]');
    dragRegions.forEach(el => {
      el.addEventListener('mousedown', (e) => {
        if (e.button !== 0) return;
        if (e.target.closest('button, input, textarea, select')) return;
        tauriWindow.getCurrentWindow().startDragging();
      });
    });
  }

  const tauriListen = window.__TAURI__?.event?.listen ?? window.__TAURI_INTERNALS__?.event?.listen;
  if (tauriListen) {
    tauriListen("plexi-menu-command", (event) => runCommand(event.payload));
  }

  window.addEventListener("keydown", (event) => {
    if (event.metaKey && event.key === "q" && !event.repeat) {
      event.preventDefault();
      const invoke = window.__TAURI__?.core?.invoke ?? window.__TAURI_INTERNALS__?.invoke;
      if (invoke) invoke("quit_app");
      return;
    }

    if (event.key === "Escape" && uiState.contextModalOpen) {
      event.preventDefault();
      closeContextModal();
      return;
    }

    if (event.key === "Escape" && uiState.overviewOpen) {
      event.preventDefault();
      closeOverview();
      render();
      return;
    }

    if (!event.defaultPrevented) {
      handleShortcutKeydown(event);
    }
  }, true);
}

bindUiEvents();
syncViewportMetrics();
window.addEventListener("resize", syncViewportMetrics);
window.visualViewport?.addEventListener("resize", syncViewportMetrics);
window.addEventListener("beforeunload", () => {
  state.panels.forEach((panel) => closePanelSession(panel.id));
  void sessionBridge.reset?.();
});

window.__PLEXI_DEBUG__ = {
  getState: () => clone(state),
  getTerminalProfile: () => getTerminalProfile(getActivePanel(state)?.fontSize),
  getPanelBuffer: (panelId = state.activePanelId) => panelBuffers.get(panelId) || "",
  runCommand,
  deleteContextFromUi: deleteContext,
  reset: () => {
    state.panels.forEach((panel) => closePanelSession(panel.id));
    void sessionBridge.reset();
    disposeAllPaneRuntimes();
    panelMeta.clear();
    panelBuffers.clear();
    outputSequences.clear();
    state = bootDefaultState();
    closeContextModal();
    closeOverview();
    saveState();
    render();
  },
};

async function initializeApp() {
  const [info, hydrated] = await Promise.all([
    sessionBridge.getBackendInfo(),
    hydrateWorkspaceState(sessionBridge),
    loadConfig(sessionBridge),
  ]);

  backendInfo = info;
  state = hydrated.state;
  updateWorkspaceStorage(hydrated.storage);

  if (hydrated.warning) {
    showToast(hydrated.warning);
  }

  if ((sessionBridge.mode === "live" || sessionBridge.mode === "tauri") && !hydrated.storage) {
    updateWorkspaceStorage({
      path: "Workspace file unavailable",
      source: "disk",
    });
  }

  if ((sessionBridge.mode === "live" || sessionBridge.mode === "tauri") && hydrated.storage && hydrated.state.lastAction === "Ready") {
    saveState();
  }

  const warnings = getConfigWarnings();
  if (warnings.length > 0) {
    console.warn("Config warnings:", warnings);
    showToast(`Config: ${warnings[0]}${warnings.length > 1 ? ` (+${warnings.length - 1} more)` : ""}`);
  }

  syncViewportMetrics();
  await render();
}

void initializeApp();
