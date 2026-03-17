import { ASSET_CANDIDATES, TERMINAL_PROFILE, isMacOS } from "./app-constants.js";

let xtermStatus = "loading";
let terminalFontReady = null;
const MIN_TERMINAL_FONT_SIZE = 10;
const MAX_TERMINAL_FONT_SIZE = 28;
const TERMINAL_FONT_STEP = 1;

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
      script.async = false;
      script.onload = () => resolve(true);
      script.onerror = () => {
        script.remove();
        resolve(false);
      };
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

  if (!window.WebLinksAddon) {
    await loadScript(ASSET_CANDIDATES.webLinksJs);
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

export function clampTerminalFontSize(fontSize) {
  return Math.min(MAX_TERMINAL_FONT_SIZE, Math.max(MIN_TERMINAL_FONT_SIZE, Math.round(fontSize)));
}

export function createTerminalRuntime({
  panel,
  mountNode,
  onData,
  onShortcut,
  onResize,
  replayBuffer,
  onLinkClick,
  interactive = true,
}) {
  const terminal = new window.Terminal({
    ...TERMINAL_PROFILE,
    cursorBlink: interactive && TERMINAL_PROFILE.cursorBlink,
    disableStdin: !interactive,
    fontSize: clampTerminalFontSize(panel?.fontSize ?? TERMINAL_PROFILE.fontSize),
    macOptionIsMeta: isMacOS,
    overviewRulerLanes: 0,
    overviewRuler: { width: 1 },
  });
  const fitAddon = new window.FitAddon.FitAddon();
  const webLinksAddon = window.WebLinksAddon ? new window.WebLinksAddon.WebLinksAddon(
    (event, uri) => {
      if (event.metaKey || event.ctrlKey) {
        event.preventDefault();
        onLinkClick?.(uri);
      }
    }
  ) : null;

  terminal.loadAddon(fitAddon);
  if (webLinksAddon) {
    terminal.loadAddon(webLinksAddon);
  }
  
  terminal.open(mountNode);
  mountNode.dataset.terminalFontFamily = TERMINAL_PROFILE.fontFamily;

  function safeFit() {
    if (!mountNode.isConnected) {
      return false;
    }

    const rect = mountNode.getBoundingClientRect();
    if (rect.width < 8 || rect.height < 8) {
      return false;
    }

    try {
      fitAddon.fit();
      return terminal.cols > 0 && terminal.rows > 0;
    } catch (_error) {
      return false;
    }
  }

  const runtime = {
    interactive,
    panel,
    mountNode,
    terminal,
    fitAddon,
    webLinksAddon,
    pendingWrites: [],
    writeFrame: 0,
    needsScrollToBottom: false,
    resizeFrame: 0,
    resizeObserver: null,
    resizeHandler: () => {
      if (runtime.resizeFrame) {
        window.cancelAnimationFrame(runtime.resizeFrame);
      }
      runtime.resizeFrame = window.requestAnimationFrame(() => {
        runtime.resizeFrame = 0;
        if (safeFit()) {
          onResize(runtime);
        }
      });
    },
    enqueueWrite(chunk, { scrollToBottom = false } = {}) {
      if (!chunk) {
        return;
      }

      runtime.pendingWrites.push(chunk);
      runtime.needsScrollToBottom = runtime.needsScrollToBottom || scrollToBottom;

      if (runtime.writeFrame) {
        return;
      }

      runtime.writeFrame = window.requestAnimationFrame(() => {
        runtime.writeFrame = 0;
        const nextChunk = runtime.pendingWrites.join("");
        runtime.pendingWrites.length = 0;
        const shouldScroll = runtime.needsScrollToBottom;
        runtime.needsScrollToBottom = false;
        terminal.write(nextChunk, () => {
          if (shouldScroll) {
            terminal.scrollToBottom?.();
          }
        });
      });
    },
    dispose() {
      if (runtime.writeFrame) {
        window.cancelAnimationFrame(runtime.writeFrame);
      }
      if (runtime.resizeFrame) {
        window.cancelAnimationFrame(runtime.resizeFrame);
      }
      clearTimeout(scrollHideTimer);
      runtime.resizeObserver?.disconnect?.();
      window.removeEventListener("resize", runtime.resizeHandler);
      terminal.dispose();
    },
  };

  if (interactive) {
    terminal.attachCustomKeyEventHandler((event) => {
      if (onShortcut(event, runtime) === false) {
        return false;
      }

      const nativeInput = resolveNativeTerminalInput(event);
      if (!nativeInput) {
        return true;
      }

      event.preventDefault();
      onData(runtime, nativeInput);
      return false;
    });
    terminal.onData((rawData) => {
      onData(runtime, rawData);
    });
  }

  let scrollHideTimer = 0;
  terminal.onScroll(() => {
    mountNode.classList.add("terminal-mount--scrolling");
    clearTimeout(scrollHideTimer);
    scrollHideTimer = setTimeout(() => {
      mountNode.classList.remove("terminal-mount--scrolling");
    }, 1500);
  });

  window.addEventListener("resize", runtime.resizeHandler);
  if (window.ResizeObserver) {
    runtime.resizeObserver = new window.ResizeObserver(() => {
      runtime.resizeHandler();
    });
    runtime.resizeObserver.observe(mountNode);
  }
  runtime.resizeHandler();
  replayBuffer(runtime);

  return runtime;
}

export function getTerminalProfile(fontSize = TERMINAL_PROFILE.fontSize) {
  return { ...TERMINAL_PROFILE, fontSize: clampTerminalFontSize(fontSize) };
}

export function getTerminalFontSize(panel = null) {
  return clampTerminalFontSize(panel?.fontSize ?? TERMINAL_PROFILE.fontSize);
}

export function adjustTerminalFontSize(delta, runtime = null, currentFontSize = TERMINAL_PROFILE.fontSize) {
  const nextFontSize = clampTerminalFontSize(currentFontSize + delta);

  if (runtime?.terminal?.options) {
    runtime.terminal.options.fontSize = nextFontSize;
    try {
      runtime.fitAddon?.fit?.();
    } catch (_error) {
      // Ignore transient fit races during resize or remount.
    }
  }

  return nextFontSize;
}

export function getTerminalZoomStep() {
  return TERMINAL_FONT_STEP;
}

export function resolveNativeTerminalInput(event) {
  if (!isMacOS || event.type !== "keydown" || event.defaultPrevented || event.ctrlKey) {
    return null;
  }

  if (event.metaKey && !event.altKey && !event.shiftKey) {
    if (event.key === "ArrowLeft" || event.key === "Home") {
      return "\u0001";
    }

    if (event.key === "ArrowRight" || event.key === "End") {
      return "\u0005";
    }

    if (event.key === "Backspace" || event.key === "Delete") {
      return "\u0015";
    }
  }

  if (event.altKey && !event.metaKey && !event.shiftKey) {
    if (event.key === "ArrowLeft") {
      return "\u001bb";
    }

    if (event.key === "ArrowRight") {
      return "\u001bf";
    }

    if (event.key === "Backspace") {
      return "\u001b\u007f";
    }

    if (event.key === "Delete") {
      return "\u001bd";
    }
  }

  return null;
}
