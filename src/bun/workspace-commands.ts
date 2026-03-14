import type { BrowserWindow } from "electrobun/bun";
import { WORKSPACE_COMMANDS, isWorkspaceCommand } from "../shared/commands.js";

const MENU_ACTION_TO_COMMAND = {
  "workspace:new-terminal-right": WORKSPACE_COMMANDS.newTerminalRight,
  "workspace:new-terminal-down": WORKSPACE_COMMANDS.newTerminalDown,
  "workspace:close-terminal": WORKSPACE_COMMANDS.closeTerminal,
  "workspace:save": WORKSPACE_COMMANDS.saveWorkspace,
  "workspace:toggle-overview": WORKSPACE_COMMANDS.toggleOverview,
  "workspace:toggle-sidebar": WORKSPACE_COMMANDS.toggleSidebar,
  "workspace:reset-viewport": WORKSPACE_COMMANDS.resetViewport,
  "workspace:zoom-in": WORKSPACE_COMMANDS.zoomIn,
  "workspace:zoom-out": WORKSPACE_COMMANDS.zoomOut,
  "workspace:focus-right": WORKSPACE_COMMANDS.focusRight,
  "workspace:focus-left": WORKSPACE_COMMANDS.focusLeft,
  "workspace:focus-up": WORKSPACE_COMMANDS.focusUp,
  "workspace:focus-down": WORKSPACE_COMMANDS.focusDown,
  "workspace:context-1": WORKSPACE_COMMANDS.context1,
  "workspace:context-2": WORKSPACE_COMMANDS.context2,
  "workspace:context-3": WORKSPACE_COMMANDS.context3,
  "workspace:context-4": WORKSPACE_COMMANDS.context4,
  "workspace:context-5": WORKSPACE_COMMANDS.context5,
  "workspace:context-6": WORKSPACE_COMMANDS.context6,
  "workspace:context-7": WORKSPACE_COMMANDS.context7,
  "workspace:context-8": WORKSPACE_COMMANDS.context8,
  "workspace:context-9": WORKSPACE_COMMANDS.context9,
  "workspace:next-context": WORKSPACE_COMMANDS.nextContext,
  "workspace:previous-context": WORKSPACE_COMMANDS.previousContext,
  "workspace:show-shortcuts": WORKSPACE_COMMANDS.showShortcuts,
  "workspace:edit-workspace-configuration": WORKSPACE_COMMANDS.editWorkspaceConfiguration,
} as const;

export function getWorkspaceCommandForMenuAction(action: string | undefined) {
  if (!action) {
    return null;
  }

  return MENU_ACTION_TO_COMMAND[action as keyof typeof MENU_ACTION_TO_COMMAND] || null;
}

export function dispatchWorkspaceCommand(window: BrowserWindow | null, command: string | null) {
  if (!window || !command || !isWorkspaceCommand(command)) {
    return;
  }

  const payload = JSON.stringify(command);
  window.webview.executeJavascript(
    `window.dispatchEvent(new CustomEvent("plexi:command", { detail: { command: ${payload} } }));`,
  );
}
