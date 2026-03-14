import { describe, expect, test } from "bun:test";
import { resolveTerminalKeybind } from "../../src/mainview/terminal-shortcuts.js";
import { resolveKeybind, compileKeybinds } from "../../src/shared/keybinds.js";

const primaryModifier = process.platform === "darwin"
  ? { metaKey: true }
  : { ctrlKey: true };

function createEvent(options: Partial<KeyboardEvent> & { key: string }) {
  return {
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    type: "keydown",
    code: "",
    ...options,
  } as KeyboardEvent;
}

describe("resolveTerminalKeybind", () => {
  test("treats copy as performable so ctrl-c can fall through to the terminal", () => {
    const match = resolveTerminalKeybind(createEvent({
      key: "c",
      code: "KeyC",
      ...primaryModifier,
    }), {
      hasSelection: false,
    });

    expect(match).toBeNull();
  });

  test("matches copy when a selection exists", () => {
    const match = resolveTerminalKeybind(createEvent({
      key: "c",
      code: "KeyC",
      ...primaryModifier,
    }), {
      hasSelection: true,
    });

    expect(match?.action.name).toBe("copy_to_clipboard");
    expect(match?.performable).toBe(true);
    expect(match?.consume).toBe(true);
  });

  test("matches paste as a normal consumed terminal action", () => {
    const match = resolveTerminalKeybind(createEvent({
      key: "v",
      code: "KeyV",
      ...primaryModifier,
    }), {
      hasSelection: false,
    });

    expect(match?.action.name).toBe("paste_from_clipboard");
    expect(match?.consume).toBe(true);
  });
});

describe("resolveKeybind", () => {
  test("normalizes punctuation and arrow keys from KeyboardEvent.code", () => {
    const bindings = compileKeybinds([
      "ctrl+m=toggle_minimap",
      "ctrl+/=toggle_shortcuts",
      "ctrl+left=focus_left",
      "ctrl+equal=zoom_in",
    ]);

    expect(resolveKeybind(createEvent({
      key: "m",
      code: "KeyM",
      ctrlKey: true,
    }), bindings)?.action.name).toBe("toggle_minimap");

    expect(resolveKeybind(createEvent({
      key: "/",
      code: "Slash",
      ctrlKey: true,
    }), bindings)?.action.name).toBe("toggle_shortcuts");

    expect(resolveKeybind(createEvent({
      key: "ArrowLeft",
      code: "ArrowLeft",
      ctrlKey: true,
    }), bindings)?.action.name).toBe("focus_left");

    expect(resolveKeybind(createEvent({
      key: "+",
      code: "Equal",
      ctrlKey: true,
    }), bindings)?.action.name).toBe("zoom_in");
  });

  test("supports Ghostty-style unconsumed bindings", () => {
    const bindings = compileKeybinds(["unconsumed:ctrl+j=text:\\n"]);
    const match = resolveKeybind(createEvent({
      key: "j",
      code: "KeyJ",
      ctrlKey: true,
    }), bindings);

    expect(match?.action.name).toBe("text");
    expect(match?.action.argument).toBe("\\n");
    expect(match?.consume).toBe(false);
  });

  test("ignores non-keydown events", () => {
    const bindings = compileKeybinds(["ctrl+n=new_terminal_right"]);
    const match = resolveKeybind(createEvent({
      key: "n",
      code: "KeyN",
      ctrlKey: true,
      type: "keypress",
    }), bindings);

    expect(match).toBeNull();
  });
});
