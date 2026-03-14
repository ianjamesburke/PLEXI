import { beforeEach, describe, expect, test } from "bun:test";
import {
  adjustTerminalFontSize,
  getTerminalFontSize,
  getTerminalProfile,
  getTerminalZoomStep,
} from "../../src/mainview/xterm-runtime.js";

const DEFAULT_FONT_SIZE = 14;

describe("xterm runtime zoom", () => {
  beforeEach(() => {
    adjustTerminalFontSize(DEFAULT_FONT_SIZE - getTerminalFontSize());
  });

  test("starts from the terminal profile font size", () => {
    expect(getTerminalProfile().fontSize).toBe(DEFAULT_FONT_SIZE);
    expect(getTerminalFontSize()).toBe(DEFAULT_FONT_SIZE);
  });

  test("adjusts terminal font size using the configured zoom step", () => {
    const nextFontSize = adjustTerminalFontSize(getTerminalZoomStep());

    expect(nextFontSize).toBe(DEFAULT_FONT_SIZE + 1);
    expect(getTerminalFontSize()).toBe(DEFAULT_FONT_SIZE + 1);
    expect(getTerminalProfile().fontSize).toBe(DEFAULT_FONT_SIZE + 1);
  });

  test("clamps terminal font size to the minimum bound", () => {
    const minFontSize = adjustTerminalFontSize(-200);

    expect(minFontSize).toBe(10);
    expect(getTerminalFontSize()).toBe(10);
  });

  test("clamps terminal font size to the maximum bound", () => {
    const maxFontSize = adjustTerminalFontSize(200);

    expect(maxFontSize).toBe(28);
    expect(getTerminalFontSize()).toBe(28);
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

    const nextFontSize = adjustTerminalFontSize(getTerminalZoomStep(), runtime as any);

    expect(nextFontSize).toBe(DEFAULT_FONT_SIZE + 1);
    expect(runtime.terminal.options.fontSize).toBe(DEFAULT_FONT_SIZE + 1);
    expect(fitCalls).toBe(1);
  });
});
