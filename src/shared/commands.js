export const WORKSPACE_COMMANDS = {
  newTerminalRight: "new-terminal-right",
  newTerminalDown: "new-terminal-down",
  newNodeRight: "new-node-right",
  newNodeDown: "new-node-down",
  closeTerminal: "close-terminal",
  saveWorkspace: "save-workspace",
  jumpBack: "jump-back",
  toggleSidebar: "toggle-sidebar",
  toggleMinimap: "toggle-minimap",
  toggleShortcuts: "toggle-shortcuts",
  zoomIn: "zoom-in",
  zoomOut: "zoom-out",
  focusRight: "focus-right",
  focusLeft: "focus-left",
  focusUp: "focus-up",
  focusDown: "focus-down",
  nextContext: "next-context",
  previousContext: "previous-context",
  newContext: "new-context",
  showShortcuts: "show-shortcuts",
  context1: "context-1",
  context2: "context-2",
  context3: "context-3",
  context4: "context-4",
  context5: "context-5",
  context6: "context-6",
  context7: "context-7",
  context8: "context-8",
  context9: "context-9",
};

export const CONTEXT_COMMANDS = [
  WORKSPACE_COMMANDS.context1,
  WORKSPACE_COMMANDS.context2,
  WORKSPACE_COMMANDS.context3,
  WORKSPACE_COMMANDS.context4,
  WORKSPACE_COMMANDS.context5,
  WORKSPACE_COMMANDS.context6,
  WORKSPACE_COMMANDS.context7,
  WORKSPACE_COMMANDS.context8,
  WORKSPACE_COMMANDS.context9,
];

export const MENU_ONLY_WORKSPACE_COMMANDS = new Set([
  WORKSPACE_COMMANDS.showShortcuts,
]);

export const KNOWN_WORKSPACE_COMMANDS = new Set([
  ...Object.values(WORKSPACE_COMMANDS),
]);

export function isWorkspaceCommand(value) {
  return typeof value === "string" && KNOWN_WORKSPACE_COMMANDS.has(value);
}
