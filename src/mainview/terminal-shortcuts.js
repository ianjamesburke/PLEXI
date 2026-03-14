export const TERMINAL_SHORTCUT_ACTIONS = {
  copy: "copy",
  interrupt: "interrupt",
  paste: "paste",
  pass: "pass",
};

export function resolveTerminalShortcutAction(event, { hasSelection, isMacOS }) {
  const key = event.key.toLowerCase();
  const hasMod = event.metaKey || event.ctrlKey;

  if (hasSelection && hasMod && key === "c") {
    return TERMINAL_SHORTCUT_ACTIONS.copy;
  }

  if (hasMod && key === "v") {
    return TERMINAL_SHORTCUT_ACTIONS.paste;
  }

  const isCtrlInterrupt =
    event.ctrlKey &&
    !event.metaKey &&
    !event.altKey &&
    key === "c";

  if (isCtrlInterrupt) {
    return TERMINAL_SHORTCUT_ACTIONS.interrupt;
  }

  return TERMINAL_SHORTCUT_ACTIONS.pass;
}
