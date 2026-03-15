import {
  DIRECTIONS,
  PANEL_TYPES,
  clone,
  closePanelRecord,
  createContextRecord,
  createPanelRecord,
  deleteContextRecord,
  ensureActivePanel,
  focusDirectionalPanel,
  focusPanel,
  getActivePanel,
  getBounds,
  getVisiblePanels,
  movePanelRecord,
  renameContextRecord,
  setContextIndex,
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
  getXtermStatus,
  setXtermError,
  getTerminalZoomStep,
} from "./xterm-runtime.js";

let terminalRuntime = null;
const panelBuffers = new Map();
const panelMeta = new Map();
const panelSessions = new Set();
let backendInfo = null;
let homeDirectory = null;
let state = bootDefaultState();
const uiState = {
  workspaceStorageSource: "browser",
  contextModalOpen: false,
  contextRenameIndex: null,
  contextDeleteConfirming: false,
  contextDeleteTimer: null,
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
    clearNode(dom.terminalMount);
    return;
  }

  try {
    await ensureXtermAssets();
    await ensureTerminalFont();
  } catch (error) {
    setXtermError();
    dom.terminalMount.textContent = String(error);
    dom.terminalMount.classList.add("terminal-mount--error");
    return;
  }


  dom.terminalMount.classList.remove("terminal-mount--loading", "terminal-mount--error");
  if (terminalRuntime?.panel?.id !== activePanel.id) {
    disposeRuntime();
    clearNode(dom.terminalMount);
    terminalRuntime = createTerminalRuntime({
      panel: activePanel,
      mountNode: dom.terminalMount,
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
      onLinkClick(uri) {
        if (uri) {
          void sessionBridge.openExternalUrl(uri);
        }
      },
      replayBuffer,
    });
    return;
  }

  terminalRuntime.panel = activePanel;
  void sessionBridge.resizeSession({
    panelId: activePanel.id,
    cols: terminalRuntime.terminal.cols,
    rows: terminalRuntime.terminal.rows,
  });
  terminalRuntime.terminal.focus();
}


function renderContextModal() {
  if (!dom.contextModal) {
    return;
  }

  dom.contextModal.classList.toggle("is-hidden", !uiState.contextModalOpen);

  const titleEl = dom.contextModal.querySelector("#context-modal-title");
  if (titleEl) {
    titleEl.textContent = uiState.contextRenameIndex !== null ? "Rename context" : "New context";
  }

  const deleteBtn = dom.contextDeleteButton;
  if (deleteBtn) {
    deleteBtn.classList.toggle("is-hidden", uiState.contextRenameIndex === null);
    deleteBtn.textContent = uiState.contextDeleteConfirming ? "Are you sure?" : "Delete";
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

function createTerminal(direction, cwd = null, cwdLabel = null) {
  if (state.contexts.length === 0) {
    createContextRecord(state, "");
  }

  const panel = createPanelRecord(state, { direction, cwd, cwdLabel });
  clearPanelBuffer(panel.id);
  void ensurePanelSession(panel);
  setLastAction(
    direction === DIRECTIONS.down
      ? `${panel.title} created below`
      : `${panel.title} created to the right`,
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
    if (terminalRuntime?.panel?.id === removed.id) {
      disposeRuntime();
      clearNode(dom.terminalMount);
    }
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
    case WORKSPACE_COMMANDS.closeTerminal:
      closeActiveTerminal();
      break;
    case WORKSPACE_COMMANDS.toggleSidebar:
      state.sidebarVisible = !state.sidebarVisible;
      setLastAction(state.sidebarVisible ? "Sidebar shown" : "Sidebar hidden");
      break;
    case WORKSPACE_COMMANDS.toggleMinimap:
      state.minimapVisible = !state.minimapVisible;
      setLastAction(state.minimapVisible ? "Map shown" : "Map hidden");
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
      adjustTerminalFontSize(getTerminalZoomStep(), terminalRuntime);
      setLastAction("Font size increased");
      saveState();
      break;
    case WORKSPACE_COMMANDS.zoomOut:
      adjustTerminalFontSize(-getTerminalZoomStep(), terminalRuntime);
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
    contextButton.textContent = label;

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

function buildMinimapNodes(visiblePanels, activePanel, gridElement, maxHeight) {
  const bounds = getBounds(visiblePanels);
  const width = gridElement.clientWidth || 228;
  const spanX = Math.max(bounds.width + 1, 1);
  const spanY = Math.max(bounds.height + 1, 1);
  const gutter = 10;
  const nodeSize = 18;
  const cellWidth = Math.max(nodeSize + 4, Math.min(nodeSize + 12, Math.floor((width - gutter * 2) / spanX)));
  const cellHeight = Math.max(nodeSize + 2, Math.min(nodeSize + 8, Math.floor(maxHeight / spanY)));
  const gridHeight = Math.max(88, Math.min(maxHeight + 28, spanY * cellHeight + gutter * 2));
  gridElement.style.height = `${gridHeight}px`;

  return visiblePanels.map((panel) => {
    const left = (panel.x - bounds.minX) * cellWidth + gutter;
    const top = (panel.y - bounds.minY) * cellHeight + gutter;
    const node = document.createElement("button");
    const active = panel.id === activePanel?.id ? "is-active" : "";
    const rowFocus = isRowFocusPanel(panel) ? "is-row-focus" : "";
    node.className = `minimap-node ${active} ${rowFocus}`.trim();
    node.dataset.focusPanel = panel.id;
    node.style.left = `${left}px`;
    node.style.top = `${top}px`;
    node.setAttribute("aria-label", panel.title);
    return node;
  });
}

function renderMinimap(visiblePanels, activePanel) {
  const hasVisiblePanels = visiblePanels.length > 0;
  const shouldShowOverlayMinimap = state.minimapVisible !== false && hasVisiblePanels;
  dom.minimap.classList.toggle("is-hidden", !hasVisiblePanels);
  dom.overlayMinimap.classList.toggle("is-hidden", !shouldShowOverlayMinimap);
  dom.minimapSize.textContent = `${visiblePanels.length} terminal${visiblePanels.length === 1 ? "" : "s"}`;

  if (!hasVisiblePanels) {
    clearNode(dom.minimapGrid);
    clearNode(dom.overlayMinimapGrid);
    return;
  }

  const sidebarNodes = buildMinimapNodes(visiblePanels, activePanel, dom.minimapGrid, 148);
  dom.minimapGrid.replaceChildren(...sidebarNodes);

  if (!shouldShowOverlayMinimap) {
    clearNode(dom.overlayMinimapGrid);
    return;
  }

  const overlayNodes = buildMinimapNodes(visiblePanels, activePanel, dom.overlayMinimapGrid, 120);
  dom.overlayMinimapGrid.replaceChildren(...overlayNodes);
}

function hasOpenAdjacentSpace(panel, direction) {
  if (!panel) {
    return false;
  }

  const target =
    direction === DIRECTIONS.down
      ? { x: panel.x, y: panel.y + 1 }
      : { x: panel.x + 1, y: panel.y };

  return !state.panels.some((item) =>
    item.contextIndex === panel.contextIndex && item.x === target.x && item.y === target.y);
}

function renderSparseFocusHints(visiblePanels, activePanel) {
  const canShowHints = visiblePanels.length === 1 && Boolean(activePanel) && !activePanel.hasReceivedInput;
  const rightOpen = canShowHints && hasOpenAdjacentSpace(activePanel, DIRECTIONS.right);
  const bottomOpen = canShowHints && hasOpenAdjacentSpace(activePanel, DIRECTIONS.down);
  dom.focusRightSlot?.classList.toggle("is-hidden", !rightOpen);
  dom.focusBottomSlot?.classList.toggle("is-hidden", !bottomOpen);
}

function renderFocus(activePanel) {
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
  const hasActivePanel = Boolean(activePanel);
  dom.focusPath?.classList.toggle("is-hidden", !hasActivePanel);
  dom.focusProcess?.classList.toggle("is-hidden", !hasActivePanel);
}

async function render() {
  ensurePanelSessions();
  renderContextButtons();
  renderContextModal();

  const visiblePanels = getVisiblePanels(state);
  const activePanel = ensureActivePanel(state);
  renderFocus(activePanel);
  renderToolbarState(activePanel);
  renderMinimap(visiblePanels, activePanel);
  renderSparseFocusHints(visiblePanels, activePanel);
  dom.appShell.classList.toggle("app-shell--sidebar-hidden", !state.sidebarVisible);
  dom.sidebar?.setAttribute("aria-hidden", String(!state.sidebarVisible));

  dom.focusShell.classList.toggle("is-hidden", visiblePanels.length === 0);
  dom.emptyShell.classList.toggle("is-hidden", visiblePanels.length !== 0);
  dom.shortcutsOverlay.classList.toggle("is-hidden", !state.shortcutsVisible);


  if (activePanel) {
    await mountActiveTerminal(activePanel);
  }
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
  dom.contextCloseButton?.addEventListener("click", closeContextModal);
  
  dom.contextDeleteButton?.addEventListener("click", (event) => {
    event.preventDefault();
    deleteContext();
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
  deleteContextFromUi: deleteContext,
  reset: () => {
    state.panels.forEach((panel) => closePanelSession(panel.id));
    void sessionBridge.reset();
    panelMeta.clear();
    panelBuffers.clear();
    state = bootDefaultState();
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

  if (sessionBridge.mode === "live" && hydrated.storage && hydrated.state.lastAction === "Ready") {
    saveState();
  }

  syncViewportMetrics();
  await render();
}

void initializeApp();
