import { resolveKeybind } from "../shared/keybinds.js";
import { TERMINAL_KEYBINDS } from "./keybind-config.js";

export function resolveTerminalKeybind(event, { hasSelection }) {
  return resolveKeybind(event, TERMINAL_KEYBINDS, {
    canPerform(action) {
      return action.name === "copy_to_clipboard" ? hasSelection : true;
    },
  });
}
