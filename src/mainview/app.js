import {
  CONTEXTS,
  DIRECTIONS,
  PANEL_TYPES,
  adjustZoom,
  clone,
  createPanelRecord,
  ensureActivePanel,
  focusDirectionalPanel,
  focusPanel,
  getActivePanel,
  getBounds,
  getVisiblePanels,
  makeDefaultState,
  movePanelRecord,
  panCamera,
  resetViewport,
  setContextIndex,
  toggleMode,
  closePanelRecord,
} from "../shared/workspace-state.js";
import { createSessionBridge } from "./session-bridge.js";
import { resolveTerminalShortcutAction, TERMINAL_SHORTCUT_ACTIONS } from "./terminal-shortcuts.js";

const STORAGE_KEY = "plexi.workspace.v2";
const MAX_BUFFER_CHARS = 120000;
const platformName = navigator.userAgentData?.platform || navigator.platform || navigator.userAgent;
const isMacOS = /\bMac/i.test(platformName);
let terminalRuntime = null;
const panelBuffers = new Map();
const panelMeta = new Map();
const panelSessions = new Set();
let backendInfo = null;

const appShell = document.getElementById("app-shell");
const sidebar = document.getElementById("sidebar");
const stage = document.getElementById("stage");
const focusShell = document.getElementById("focus-shell");
const overviewShell = document.getElementById("overview-shell");
const emptyShell = document.getElementById("empty-shell");
const terminalMount = document.getElementById("terminal-mount");
const overviewCanvas = document.getElementById("overview-canvas");
const minimap = document.getElementById("minimap");
const minimapGrid = document.getElementById("minimap-grid");
const minimapSize = document.getElementById("minimap-size");
const shortcutsOverlay = document.getElementById("shortcuts-overlay");

const focusTitle = document.getElementById("focus-title");
const focusPath = document.getElementById("focus-path");
const focusPosition = document.getElementById("focus-position");
const toolbarContext = document.getElementById("toolbar-context");
const engineLabel = document.getElementById("engine-label");
const modeLabel = document.getElementById("mode-label");
const activeLabel = document.getElementById("active-label");

const statusReady = document.getElementById("status-ready");
const statusContext = document.getElementById("status-context");
const statusPanels = document.getElementById("status-panels");
const statusPosition = document.getElementById("status-position");
const statusZoom = document.getElementById("status-zoom");

const ASSET_CANDIDATES = {
  xtermCss: [
    "./vendor/xterm/xterm.css",
    "../../node_modules/xterm/css/xterm.css",
  ],
  xtermJs: [
    "./vendor/xterm/xterm.js",
    "../../node_modules/xterm/lib/xterm.js",
  ],
  fitJs: [
    "./vendor/xterm/addon-fit.js",
    "../../node_modules/@xterm/addon-fit/lib/addon-fit.js",
  ],
};
const TERMINAL_FONT_FAMILY = [
  '"Plexi Terminal"',
  '"JetBrainsMono Nerd Font Mono"',
  '"JetBrains Mono"',
  '"Symbols Nerd Font Mono"',
  '"MesloLGM Nerd Font Mono"',
  '"MesloLGSDZ Nerd Font Mono"',
  '"Hack Nerd Font Mono"',
  '"0xProto Nerd Font Mono"',
  '"Menlo"',
  '"Monaco"',
  "monospace",
].join(", ");
const TERMINAL_PROFILE = {
  cursorBlink: true,
  convertEol: false,
  fontFamily: TERMINAL_FONT_FAMILY,
  fontSize: 14,
  fontWeight: "400",
  fontWeightBold: "600",
  letterSpacing: 0,
  lineHeight: 1,
  drawBoldTextInBrightColors: false,
  theme: {
    background: "#0d0f13",
    foreground: "#f3f5f7",
    cursor: "#d57936",
    selectionBackground: "rgba(213, 121, 54, 0.3)",
    black: "#0d0f13",
    brightBlack: "#66707b",
    red: "#ef8b7b",
    brightRed: "#f0a79c",
    green: "#91c27a",
    brightGreen: "#acd494",
    yellow: "#d7b36d",
    brightYellow: "#ebca8d",
    blue: "#7da3d8",
    brightBlue: "#9db8e4",
    magenta: "#bc8ed8",
    brightMagenta: "#cea8e4",
    cyan: "#6cb8bd",
    brightCyan: "#8cccd0",
    white: "#d8dde3",
    brightWhite: "#ffffff",
  },
};

let xtermStatus = "loading";
let terminalFontReady = null;
let state = loadState();
document.documentElement.style.setProperty("--plexi-font-mono", TERMINAL_FONT_FAMILY);
document.body.classList.toggle("platform-macos", isMacOS);
const sessionBridge = createSessionBridge({
  onStarted(message) {
    panelMeta.set(message.panelId, {
      shellName: message.shellName,
      shellPath: message.shellPath,
      backend: message.backend,
    });
    const panel = state.panels.find((item) => item.id === message.panelId);
    if (panel) {
      panel.cwd = message.cwdLabel;
      saveState();
      render();
    }
  },
  onOutput(message) {
    appendPanelBuffer(message.panelId, message.data);

    if (terminalRuntime?.panel?.id === message.panelId) {
      terminalRuntime.terminal.write(message.data);
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

function loadState() {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);

    if (!raw) {
      return bootDefaultState();
    }

    const parsed = JSON.parse(raw);
    const nextState = {
      ...makeDefaultState(),
      ...parsed,
      panels: Array.isArray(parsed.panels)
        ? parsed.panels.map((panel) => ({
          ...panel,
          cwd: panel.cwd || "~",
          transcript: [],
        }))
        : [],
    };
    ensureActivePanel(nextState);

    if (getVisiblePanels(nextState).length === 0) {
      return bootDefaultState();
    }

    return nextState;
  } catch (_error) {
    return bootDefaultState();
  }
}

function bootDefaultState() {
  const nextState = makeDefaultState();
  createPanelRecord(nextState, { direction: DIRECTIONS.right });
  nextState.lastAction = "Terminal 1 ready";
  return nextState;
}

function saveState() {
  const serialized = {
    ...state,
    panels: state.panels.map(({ transcript, ...panel }) => panel),
  };
  window.localStorage.setItem(STORAGE_KEY, JSON.stringify(serialized));
}

function setLastAction(label) {
  state.lastAction = label;
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

async function copySelection(runtime) {
  const selection = runtime?.terminal?.getSelection?.();

  if (!selection) {
    return;
  }

  try {
    await navigator.clipboard.writeText(selection);
    setLastAction("Selection copied");
    saveState();
    render();
  } catch (_error) {
    setLastAction("Clipboard copy failed");
    saveState();
    render();
  }
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
    saveState();
    render();
  } catch (_error) {
    setLastAction("Clipboard paste failed");
    saveState();
    render();
  }
}

async function ensureTerminalFont() {
  if (!document.fonts?.load) {
    return;
  }

  if (!terminalFontReady) {
    terminalFontReady = Promise.all([
      document.fonts.load('400 14px "Plexi Terminal"'),
      document.fonts.load('600 14px "Plexi Terminal"'),
    ]).catch(() => {});
  }

  await terminalFontReady;
}

function createRuntime(panel, mountNode) {
  const terminal = new window.Terminal(TERMINAL_PROFILE);
  const fitAddon = new window.FitAddon.FitAddon();
  terminal.loadAddon(fitAddon);
  terminal.open(mountNode);
  mountNode.dataset.terminalFontFamily = TERMINAL_PROFILE.fontFamily;
  fitAddon.fit();

  const runtime = {
    panel,
    terminal,
    fitAddon,
    resizeHandler: () => {
      fitAddon.fit();
      void sessionBridge.resizeSession({
        panelId: runtime.panel.id,
        cols: terminal.cols,
        rows: terminal.rows,
      });
    },
    dispose() {
      terminal.dispose();
    },
  };

  terminal.attachCustomKeyEventHandler((event) => {
    const action = resolveTerminalShortcutAction(event, {
      hasSelection: Boolean(terminal.getSelection()),
      isMacOS,
    });

    if (action === TERMINAL_SHORTCUT_ACTIONS.copy) {
      event.preventDefault();
      void copySelection(runtime);
      return false;
    }

    if (action === TERMINAL_SHORTCUT_ACTIONS.paste) {
      event.preventDefault();
      void pasteClipboard(runtime);
      return false;
    }

    if (action === TERMINAL_SHORTCUT_ACTIONS.interrupt) {
      event.preventDefault();
      void sessionBridge.writeToSession({
        panelId: runtime.panel.id,
        data: "\u0003",
      });
      return false;
    }

    return !handleShortcutKeydown(event);
  });

  terminal.onData((rawData) => {
    void sessionBridge.writeToSession({
      panelId: runtime.panel.id,
      data: rawData,
    });
  });

  window.addEventListener("resize", runtime.resizeHandler);
  replayBuffer(runtime);
  void sessionBridge.resizeSession({
    panelId: runtime.panel.id,
    cols: terminal.cols,
    rows: terminal.rows,
  });
  terminal.focus();
  return runtime;
}

function disposeRuntime() {
  if (!terminalRuntime) {
    return;
  }

  window.removeEventListener("resize", terminalRuntime.resizeHandler);
  terminalRuntime.dispose();
  terminalRuntime = null;
}

async function loadStylesheet(candidates) {
  if (document.querySelector('link[data-plexi-xterm="true"]')) {
    return;
  }

  for (const href of candidates) {
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = href;
    link.dataset.plexiXterm = "true";

    const loaded = await new Promise((resolve) => {
      link.onload = () => resolve(true);
      link.onerror = () => resolve(false);
      document.head.append(link);
    });

    if (loaded) {
      return;
    }

    link.remove();
  }

  throw new Error("Unable to load xterm stylesheet");
}

async function loadScript(candidates) {
  for (const src of candidates) {
    const loaded = await new Promise((resolve) => {
      const script = document.createElement("script");
      script.src = src;
      script.onload = () => resolve(true);
      script.onerror = () => resolve(false);
      document.head.append(script);
    });

    if (loaded) {
      return;
    }
  }

  throw new Error(`Unable to load script: ${candidates.join(", ")}`);
}

async function ensureXterm() {
  if (xtermStatus === "ready") {
    return;
  }

  try {
    await loadStylesheet(ASSET_CANDIDATES.xtermCss);

    if (!window.Terminal) {
      await loadScript(ASSET_CANDIDATES.xtermJs);
    }

    if (!window.FitAddon) {
      await loadScript(ASSET_CANDIDATES.fitJs);
    }

    xtermStatus = "ready";
    if (engineLabel) {
      engineLabel.textContent = "xterm.js ready";
    }
  } catch (error) {
    xtermStatus = "error";
    if (engineLabel) {
      engineLabel.textContent = "xterm.js failed";
    }
    terminalMount.textContent = String(error);
    terminalMount.classList.add("terminal-mount--error");
  }
}

function updateEngineLabel(activePanel) {
  if (xtermStatus === "error") {
    return;
  }

  const activeMeta = activePanel ? panelMeta.get(activePanel.id) : null;

  if (activeMeta?.shellName) {
    if (engineLabel) {
      engineLabel.textContent = `xterm.js + ${activeMeta.shellName}`;
    }
    return;
  }

  if (backendInfo?.shellName) {
    if (engineLabel) {
      engineLabel.textContent = `xterm.js + ${backendInfo.shellName}`;
    }
    return;
  }

  if (backendInfo?.backend === "mock") {
    if (engineLabel) {
      engineLabel.textContent = "xterm.js + mock shell";
    }
    return;
  }

  if (engineLabel) {
    engineLabel.textContent = xtermStatus === "ready" ? "xterm.js ready" : "Loading xterm.js";
  }
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

function createTerminal(direction) {
  const panel = createPanelRecord(state, { direction });
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
    closePanelSession(removed.id);
    setLastAction(`${removed.title} closed`);
  }

  if (getVisiblePanels(state).length === 0) {
    createTerminal(DIRECTIONS.right);
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

  const mod = event.metaKey || event.ctrlKey;
  const key = event.key.toLowerCase();
  const isArrowKey = ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key);

  if (mod && key === "n") {
    event.preventDefault();
    runCommand(event.shiftKey ? "new-terminal-down" : "new-terminal-right");
    return true;
  }

  if (mod && key === "w") {
    event.preventDefault();
    runCommand("close-terminal");
    return true;
  }

  if (mod && key === "s") {
    event.preventDefault();
    runCommand("save-workspace");
    return true;
  }

  if (mod && key === "b") {
    event.preventDefault();
    runCommand("toggle-sidebar");
    return true;
  }

  if (mod && /^[1-4]$/.test(event.key)) {
    event.preventDefault();
    setContextIndex(state, Number(event.key) - 1);
    setLastAction(`Context ${CONTEXTS[state.activeContextIndex]}`);
    saveState();
    render();
    return true;
  }

  if (mod && event.code === "Slash") {
    event.preventDefault();
    runCommand("toggle-shortcuts");
    return true;
  }

  if (mod && event.shiftKey && event.code === "KeyO") {
    event.preventDefault();
    runCommand("toggle-overview");
    return true;
  }

  if (mod && (event.code === "Equal" || key === "+")) {
    event.preventDefault();
    runCommand("zoom-in");
    return true;
  }

  if (mod && (event.code === "Minus" || key === "-")) {
    event.preventDefault();
    runCommand("zoom-out");
    return true;
  }

  if (state.mode === "overview" && mod && event.shiftKey && isArrowKey) {
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

  if (mod && isArrowKey) {
    event.preventDefault();

    if (event.key === "ArrowLeft") {
      handleDirectionalFocus(DIRECTIONS.left);
    } else if (event.key === "ArrowRight") {
      handleDirectionalFocus(DIRECTIONS.right);
    } else if (event.key === "ArrowUp") {
      handleDirectionalFocus(DIRECTIONS.up);
    } else if (event.key === "ArrowDown") {
      handleDirectionalFocus(DIRECTIONS.down);
    }

    saveState();
    render();
    return true;
  }

  if (state.mode === "overview" && isArrowKey && !mod) {
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
  switch (command) {
    case "new-terminal-right":
      createTerminal(DIRECTIONS.right);
      break;
    case "new-terminal-down":
      createTerminal(DIRECTIONS.down);
      break;
    case "close-terminal":
      closeActiveTerminal();
      break;
    case "toggle-overview":
      setLastAction(toggleMode(state) === "overview" ? "Overview opened" : "Focus mode");
      break;
    case "toggle-sidebar":
      state.sidebarVisible = !state.sidebarVisible;
      setLastAction(state.sidebarVisible ? "Sidebar shown" : "Sidebar hidden");
      break;
    case "toggle-shortcuts":
      toggleShortcuts();
      break;
    case "save-workspace":
      setLastAction("Workspace saved");
      break;
    case "focus-left":
      handleDirectionalFocus(DIRECTIONS.left);
      break;
    case "focus-right":
      handleDirectionalFocus(DIRECTIONS.right);
      break;
    case "focus-up":
      handleDirectionalFocus(DIRECTIONS.up);
      break;
    case "focus-down":
      handleDirectionalFocus(DIRECTIONS.down);
      break;
    case "zoom-in":
      adjustZoom(state, 0.1);
      setLastAction(`Overview zoom ${Math.round(state.camera.zoom * 100)}%`);
      break;
    case "zoom-out":
      adjustZoom(state, -0.1);
      setLastAction(`Overview zoom ${Math.round(state.camera.zoom * 100)}%`);
      break;
    case "reset-viewport":
      resetViewport(state);
      setLastAction("Overview recentered");
      break;
    case "next-context":
      setContextIndex(state, (state.activeContextIndex + 1) % CONTEXTS.length);
      setLastAction(`Context ${CONTEXTS[state.activeContextIndex]}`);
      break;
    case "previous-context":
      setContextIndex(state, (state.activeContextIndex - 1 + CONTEXTS.length) % CONTEXTS.length);
      setLastAction(`Context ${CONTEXTS[state.activeContextIndex]}`);
      break;
    case "context-1":
    case "context-2":
    case "context-3":
    case "context-4":
      setContextIndex(state, Number(command.slice(-1)) - 1);
      setLastAction(`Context ${CONTEXTS[state.activeContextIndex]}`);
      break;
    default:
      break;
  }

  saveState();
  render();
}

function renderContextButtons() {
  document.querySelectorAll("[data-context-index]").forEach((button) => {
    const index = Number(button.getAttribute("data-context-index"));
    button.classList.toggle("active", index === state.activeContextIndex);
  });
}

function renderMinimap(visiblePanels, activePanel) {
  minimap.classList.toggle("is-hidden", visiblePanels.length === 0);
  minimapSize.textContent = `${visiblePanels.length} terminal${visiblePanels.length === 1 ? "" : "s"}`;

  if (visiblePanels.length === 0) {
    minimapGrid.innerHTML = "";
    return;
  }

  const bounds = getBounds(visiblePanels);
  const width = minimapGrid.clientWidth || 228;
  const spanX = Math.max(bounds.width + 1, 1);
  const spanY = Math.max(bounds.height + 1, 1);
  const gutter = 10;
  const cellWidth = Math.max(12, Math.min(18, Math.floor((width - gutter * 2) / spanX)));
  const cellHeight = Math.max(12, Math.min(16, Math.floor(140 / spanY)));
  const gridHeight = Math.max(88, Math.min(176, spanY * cellHeight + gutter * 2));
  minimapGrid.style.height = `${gridHeight}px`;

  minimapGrid.innerHTML = visiblePanels
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
    overviewCanvas.innerHTML = "";
    return;
  }

  const bounds = getBounds(visiblePanels);
  const cell = 240;
  const offsetX = 120;
  const offsetY = 80;

  overviewCanvas.innerHTML = `
    <div class="overview-viewport" style="transform: translate(${state.camera.x}px, ${state.camera.y}px) scale(${state.camera.zoom});">
      ${visiblePanels
        .map((panel) => {
          const left = (panel.x - bounds.minX) * cell + offsetX;
          const top = (panel.y - bounds.minY) * 150 + offsetY;
          const active = panel.id === activePanel?.id ? "is-active" : "";

          return `
            <article class="overview-node ${active}" data-focus-panel="${panel.id}" style="left:${left}px;top:${top}px;">
              <h3>${panel.title}</h3>
              <p>${PANEL_TYPES[panel.type].summary}</p>
              <div class="overview-node-meta">
                <span>${panel.x}, ${panel.y}</span>
                <span>${panel.cwd}</span>
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
    focusTitle.textContent = "No active terminal";
    focusPath.textContent = `Context ${CONTEXTS[state.activeContextIndex]}`;
    focusPosition.textContent = "0, 0";
    return;
  }

  focusTitle.textContent = activePanel.title;
  focusPath.textContent = activePanel.cwd;
  focusPosition.textContent = `${activePanel.x}, ${activePanel.y}`;
}

async function mountActiveTerminal(activePanel) {
  if (!activePanel || activePanel.type !== "terminal") {
    terminalMount.innerHTML = "";
    return;
  }

  await ensureXterm();
  await ensureTerminalFont();

  if (xtermStatus !== "ready") {
    return;
  }

  terminalMount.classList.remove("terminal-mount--loading", "terminal-mount--error");
  if (!terminalRuntime) {
    terminalMount.innerHTML = "";
    terminalRuntime = createRuntime(activePanel, terminalMount);
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

async function render() {
  ensurePanelSessions();
  renderContextButtons();

  const visiblePanels = getVisiblePanels(state);
  const activePanel = ensureActivePanel(state);
  updateEngineLabel(activePanel);
  renderFocus(activePanel);
  renderOverview(visiblePanels, activePanel);
  renderMinimap(visiblePanels, activePanel);
  appShell.classList.toggle("app-shell--sidebar-hidden", !state.sidebarVisible);
  sidebar?.setAttribute("aria-hidden", String(!state.sidebarVisible));

  focusShell.classList.toggle("is-hidden", state.mode !== "focus" || visiblePanels.length === 0);
  overviewShell.classList.toggle("is-hidden", state.mode !== "overview" || visiblePanels.length === 0);
  emptyShell.classList.toggle("is-hidden", visiblePanels.length !== 0);
  shortcutsOverlay.classList.toggle("is-hidden", !state.shortcutsVisible);

  if (modeLabel) {
    modeLabel.textContent = state.mode === "focus" ? "Focus" : "Overview";
  }
  if (activeLabel) {
    activeLabel.textContent = activePanel ? activePanel.title : "None";
  }
  if (toolbarContext) {
    toolbarContext.textContent = CONTEXTS[state.activeContextIndex];
  }

  if (statusReady) {
    statusReady.textContent = state.lastAction;
  }
  if (statusContext) {
    statusContext.textContent = `Context: ${CONTEXTS[state.activeContextIndex]}`;
  }
  if (statusPanels) {
    statusPanels.textContent = `${visiblePanels.length} terminal${visiblePanels.length === 1 ? "" : "s"}`;
  }
  if (statusPosition) {
    statusPosition.textContent = activePanel ? `${activePanel.x}, ${activePanel.y}` : "0, 0";
  }
  if (statusZoom) {
    statusZoom.textContent = `${Math.round(state.camera.zoom * 100)}%`;
  }

  document.querySelectorAll("[data-focus-panel]").forEach((node) => {
    node.addEventListener("click", () => {
      const panelId = node.getAttribute("data-focus-panel");
      focusPanel(state, panelId);
      state.mode = "focus";
      setLastAction(`Focused ${getActivePanel(state)?.title}`);
      saveState();
      render();
    });
  });

  if (state.mode === "focus" && activePanel) {
    await mountActiveTerminal(activePanel);
  }

  stage.dataset.mode = state.mode;
}

document.querySelectorAll("[data-command]").forEach((button) => {
  button.addEventListener("click", () => {
    runCommand(button.getAttribute("data-command"));
  });
});

document.querySelectorAll("[data-context-index]").forEach((button) => {
  button.addEventListener("click", () => {
    setContextIndex(state, Number(button.getAttribute("data-context-index")));
    setLastAction(`Context ${CONTEXTS[state.activeContextIndex]}`);
    saveState();
    render();
  });
});

window.addEventListener("plexi:command", (event) => {
  const command = event.detail?.command;

  if (command) {
    runCommand(command);
  }
});

window.addEventListener("keydown", (event) => {
  if (!event.defaultPrevented) {
    handleShortcutKeydown(event);
  }
});

window.__PLEXI_DEBUG__ = {
  getState: () => clone(state),
  getTerminalProfile: () => ({ ...TERMINAL_PROFILE }),
  runCommand,
  reset: () => {
    state.panels.forEach((panel) => closePanelSession(panel.id));
    void sessionBridge.reset();
    panelMeta.clear();
    panelBuffers.clear();
    state = bootDefaultState();
    saveState();
    render();
  },
};

syncViewportMetrics();
window.addEventListener("resize", syncViewportMetrics);
window.visualViewport?.addEventListener("resize", syncViewportMetrics);

void sessionBridge.getBackendInfo().then((info) => {
  backendInfo = info;
  syncViewportMetrics();
  updateEngineLabel(getActivePanel(state));
  render();
});

render();
console.log("Plexi terminal workspace loaded");
