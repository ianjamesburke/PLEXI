import { ASSET_CANDIDATES, TERMINAL_PROFILE } from "./app-constants.js";

let xtermStatus = "loading";
let terminalFontReady = null;

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

export async function ensureTerminalFont() {
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

export async function ensureXtermAssets() {
  if (xtermStatus === "ready") {
    return { status: xtermStatus };
  }

  await loadStylesheet(ASSET_CANDIDATES.xtermCss);

  if (!window.Terminal) {
    await loadScript(ASSET_CANDIDATES.xtermJs);
  }

  if (!window.FitAddon) {
    await loadScript(ASSET_CANDIDATES.fitJs);
  }

  xtermStatus = "ready";
  return { status: xtermStatus };
}

export function getXtermStatus() {
  return xtermStatus;
}

export function setXtermError() {
  xtermStatus = "error";
}

export function createTerminalRuntime({ panel, mountNode, onData, onShortcut, onResize, replayBuffer }) {
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
      onResize(runtime);
    },
    dispose() {
      window.removeEventListener("resize", runtime.resizeHandler);
      terminal.dispose();
    },
  };

  terminal.attachCustomKeyEventHandler((event) => onShortcut(event, runtime));
  terminal.onData((rawData) => {
    onData(runtime, rawData);
  });

  window.addEventListener("resize", runtime.resizeHandler);
  replayBuffer(runtime);
  onResize(runtime);
  terminal.focus();

  return runtime;
}

export function getTerminalProfile() {
  return { ...TERMINAL_PROFILE };
}
