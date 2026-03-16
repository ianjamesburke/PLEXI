import { describe, expect, test } from "bun:test";
import {
  adjustTerminalFontSize,
  getTerminalFontSize,
  getTerminalProfile,
  getTerminalZoomStep,
  resolveNativeTerminalInput,
} from "../../src/mainview/xterm-runtime.js";

const DEFAULT_FONT_SIZE = 14;

describe("xterm runtime zoom", () => {
  test("starts from the terminal profile font size", () => {
    expect(getTerminalProfile().fontSize).toBe(DEFAULT_FONT_SIZE);
    expect(getTerminalFontSize()).toBe(DEFAULT_FONT_SIZE);
  });

  test("adjusts terminal font size using the configured zoom step", () => {
    const nextFontSize = adjustTerminalFontSize(getTerminalZoomStep(), null, DEFAULT_FONT_SIZE);

    expect(nextFontSize).toBe(DEFAULT_FONT_SIZE + 1);
    expect(getTerminalFontSize({ fontSize: nextFontSize } as any)).toBe(DEFAULT_FONT_SIZE + 1);
    expect(getTerminalProfile(nextFontSize).fontSize).toBe(DEFAULT_FONT_SIZE + 1);
  });

  test("clamps terminal font size to the minimum bound", () => {
    const minFontSize = adjustTerminalFontSize(-200, null, DEFAULT_FONT_SIZE);

    expect(minFontSize).toBe(10);
    expect(getTerminalFontSize({ fontSize: minFontSize } as any)).toBe(10);
  });

  test("clamps terminal font size to the maximum bound", () => {
    const maxFontSize = adjustTerminalFontSize(200, null, DEFAULT_FONT_SIZE);

    expect(maxFontSize).toBe(28);
    expect(getTerminalFontSize({ fontSize: maxFontSize } as any)).toBe(28);
  });

  test("applies updated font size to an active runtime", () => {
    let fitCalls = 0;
    const runtime = {
      terminal: {
        options: {
          fontSize: DEFAULT_FONT_SIZE,
        },
      },
      fitAddon: {
        fit() {
          fitCalls += 1;
        },
      },
    };

    const nextFontSize = adjustTerminalFontSize(getTerminalZoomStep(), runtime as any, DEFAULT_FONT_SIZE);

    expect(nextFontSize).toBe(DEFAULT_FONT_SIZE + 1);
    expect(runtime.terminal.options.fontSize).toBe(DEFAULT_FONT_SIZE + 1);
    expect(fitCalls).toBe(1);
  });
});

describe("resolveNativeTerminalInput", () => {
  function createEvent(options: Partial<KeyboardEvent> & { key: string }) {
    return {
      altKey: false,
      ctrlKey: false,
      defaultPrevented: false,
      metaKey: false,
      shiftKey: false,
      type: "keydown",
      ...options,
    } as KeyboardEvent;
  }

  test("maps command-backspace to kill-line on macOS", () => {
    if (process.platform !== "darwin") {
      expect(resolveNativeTerminalInput(createEvent({
        key: "Backspace",
        metaKey: true,
      }))).toBeNull();
      return;
    }

    const sequence = resolveNativeTerminalInput(createEvent({
      key: "Backspace",
      metaKey: true,
    }));

    expect(sequence).toBe("\u0015");
  });

  test("maps command-delete to kill-line on macOS", () => {
    if (process.platform !== "darwin") {
      expect(resolveNativeTerminalInput(createEvent({
        key: "Delete",
        metaKey: true,
      }))).toBeNull();
      return;
    }

    const sequence = resolveNativeTerminalInput(createEvent({
      key: "Delete",
      metaKey: true,
    }));

    expect(sequence).toBe("\u0015");
  });

  test("maps command-left and command-right to line boundaries on macOS", () => {
    if (process.platform !== "darwin") {
      expect(resolveNativeTerminalInput(createEvent({
        key: "ArrowLeft",
        metaKey: true,
      }))).toBeNull();
      expect(resolveNativeTerminalInput(createEvent({
        key: "ArrowRight",
        metaKey: true,
      }))).toBeNull();
      return;
    }

    expect(resolveNativeTerminalInput(createEvent({
      key: "ArrowLeft",
      metaKey: true,
    }))).toBe("\u0001");

    expect(resolveNativeTerminalInput(createEvent({
      key: "ArrowRight",
      metaKey: true,
    }))).toBe("\u0005");
  });

  test("maps home and end to readline line boundaries on macOS command aliases", () => {
    if (process.platform !== "darwin") {
      expect(resolveNativeTerminalInput(createEvent({
        key: "Home",
        metaKey: true,
      }))).toBeNull();
      expect(resolveNativeTerminalInput(createEvent({
        key: "End",
        metaKey: true,
      }))).toBeNull();
      return;
    }

    expect(resolveNativeTerminalInput(createEvent({
      key: "Home",
      metaKey: true,
    }))).toBe("\u0001");

    expect(resolveNativeTerminalInput(createEvent({
      key: "End",
      metaKey: true,
    }))).toBe("\u0005");
  });

  test("maps option-left and option-right to emacs word motion on macOS", () => {
    if (process.platform !== "darwin") {
      expect(resolveNativeTerminalInput(createEvent({
        key: "ArrowLeft",
        altKey: true,
      }))).toBeNull();
      expect(resolveNativeTerminalInput(createEvent({
        key: "ArrowRight",
        altKey: true,
      }))).toBeNull();
      return;
    }

    expect(resolveNativeTerminalInput(createEvent({
      key: "ArrowLeft",
      altKey: true,
    }))).toBe("\u001bb");

    expect(resolveNativeTerminalInput(createEvent({
      key: "ArrowRight",
      altKey: true,
    }))).toBe("\u001bf");
  });

  test("maps option-delete to kill-word on macOS", () => {
    if (process.platform !== "darwin") {
      expect(resolveNativeTerminalInput(createEvent({
        key: "Delete",
        altKey: true,
      }))).toBeNull();
      return;
    }

    const sequence = resolveNativeTerminalInput(createEvent({
      key: "Delete",
      altKey: true,
    }));

    expect(sequence).toBe("\u001bd");
  });

  test("maps option-backspace to backward-kill-word on macOS", () => {
    if (process.platform !== "darwin") {
      expect(resolveNativeTerminalInput(createEvent({
        key: "Backspace",
        altKey: true,
      }))).toBeNull();
      return;
    }

    const sequence = resolveNativeTerminalInput(createEvent({
      key: "Backspace",
      altKey: true,
    }));

    expect(sequence).toBe("\u001b\u007f");
  });

  test("ignores unrelated shortcuts", () => {
    expect(resolveNativeTerminalInput(createEvent({
      key: "ArrowLeft",
      metaKey: true,
      shiftKey: true,
    }))).toBeNull();

    expect(resolveNativeTerminalInput(createEvent({
      key: "ArrowLeft",
      altKey: true,
      ctrlKey: true,
    }))).toBeNull();
  });
});
