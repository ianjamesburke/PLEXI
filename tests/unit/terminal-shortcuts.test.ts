import { describe, expect, test } from "bun:test";
import {
  resolveTerminalShortcutAction,
  TERMINAL_SHORTCUT_ACTIONS,
} from "../../src/mainview/terminal-shortcuts.js";

function createEvent(options: Partial<KeyboardEvent> & { key: string }) {
  return {
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    ...options,
  } as KeyboardEvent;
}

describe("resolveTerminalShortcutAction", () => {
  test("prefers copy when a selection exists", () => {
    const action = resolveTerminalShortcutAction(createEvent({
      key: "c",
      metaKey: true,
    }), {
      hasSelection: true,
      isMacOS: true,
    });

    expect(action).toBe(TERMINAL_SHORTCUT_ACTIONS.copy);
  });

  test("maps control-c to interrupt without a selection", () => {
    const action = resolveTerminalShortcutAction(createEvent({
      key: "c",
      ctrlKey: true,
    }), {
      hasSelection: false,
      isMacOS: false,
    });

    expect(action).toBe(TERMINAL_SHORTCUT_ACTIONS.interrupt);
  });

  test("does not treat command-c as shell interrupt on macOS", () => {
    const action = resolveTerminalShortcutAction(createEvent({
      key: "c",
      metaKey: true,
    }), {
      hasSelection: false,
      isMacOS: true,
    });

    expect(action).toBe(TERMINAL_SHORTCUT_ACTIONS.pass);
  });

  test("keeps command-c untouched outside macOS when there is no selection", () => {
    const action = resolveTerminalShortcutAction(createEvent({
      key: "c",
      metaKey: true,
    }), {
      hasSelection: false,
      isMacOS: false,
    });

    expect(action).toBe(TERMINAL_SHORTCUT_ACTIONS.pass);
  });

  test("maps mod-v to paste", () => {
    const action = resolveTerminalShortcutAction(createEvent({
      key: "v",
      ctrlKey: true,
    }), {
      hasSelection: false,
      isMacOS: false,
    });

    expect(action).toBe(TERMINAL_SHORTCUT_ACTIONS.paste);
  });
});
