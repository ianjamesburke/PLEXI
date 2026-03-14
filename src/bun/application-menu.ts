import type { ApplicationMenuItemConfig } from "electrobun/bun";
import { WORKSPACE_COMMANDS } from "../shared/commands.js";

export const createApplicationMenu = (): ApplicationMenuItemConfig[] => [
  {
    submenu: [
      { role: "hide" },
      { role: "hideOthers" },
      { role: "showAll" },
      { type: "separator" },
      { role: "quit" },
    ],
  },
  {
    label: "File",
    submenu: [
      { label: "New Terminal Right", action: "workspace:new-terminal-right", accelerator: "n" },
      { label: "New Terminal Below", action: "workspace:new-terminal-down", accelerator: "n" },
      { type: "separator" },
      { label: "Close Terminal", action: "workspace:close-terminal", accelerator: "w" },
      { label: "Close Window", role: "close" },
      { type: "separator" },
      { label: "Save Workspace", action: "workspace:save", accelerator: "s" },
      { type: "separator" },
      { label: "Quit Plexi", role: "quit" },
    ],
  },
  {
    label: "Edit",
    submenu: [
      { role: "undo" },
      { role: "redo" },
      { type: "separator" },
      { role: "cut" },
      { role: "copy" },
      { role: "paste" },
      { role: "selectAll" },
    ],
  },
  {
    label: "View",
    submenu: [
      { label: "Toggle Sidebar", action: "workspace:toggle-sidebar", accelerator: "b" },
      { label: "Toggle Map", action: "workspace:toggle-minimap" },
      { label: "Keyboard Reference", action: "workspace:show-shortcuts", accelerator: "/" },
      { type: "separator" },
      { label: "Zoom In", action: "workspace:zoom-in", accelerator: "=" },
      { label: "Zoom Out", action: "workspace:zoom-out", accelerator: "-" },
      { type: "separator" },
      { role: "toggleFullScreen" },
    ],
  },
  {
    label: "Workspace",
    submenu: [
      { label: "Focus Right", action: "workspace:focus-right" },
      { label: "Focus Left", action: "workspace:focus-left" },
      { label: "Focus Up", action: "workspace:focus-up" },
      { label: "Focus Down", action: "workspace:focus-down" },
      { type: "separator" },
      { label: "Context 1", action: `workspace:${WORKSPACE_COMMANDS.context1}`, accelerator: "1" },
      { label: "Context 2", action: `workspace:${WORKSPACE_COMMANDS.context2}`, accelerator: "2" },
      { label: "Context 3", action: `workspace:${WORKSPACE_COMMANDS.context3}`, accelerator: "3" },
      { label: "Context 4", action: `workspace:${WORKSPACE_COMMANDS.context4}`, accelerator: "4" },
      { label: "Context 5", action: `workspace:${WORKSPACE_COMMANDS.context5}`, accelerator: "5" },
      { label: "Context 6", action: `workspace:${WORKSPACE_COMMANDS.context6}`, accelerator: "6" },
      { label: "Context 7", action: `workspace:${WORKSPACE_COMMANDS.context7}`, accelerator: "7" },
      { label: "Context 8", action: `workspace:${WORKSPACE_COMMANDS.context8}`, accelerator: "8" },
      { label: "Context 9", action: `workspace:${WORKSPACE_COMMANDS.context9}`, accelerator: "9" },
      { type: "separator" },
      { label: "Next Context", action: "workspace:next-context" },
      { label: "Previous Context", action: "workspace:previous-context" },
    ],
  },
  {
    label: "Window",
    submenu: [
      { role: "minimize" },
      { role: "zoom" },
      { role: "bringAllToFront" },
    ],
  },
  {
    label: "Help",
    submenu: [{ label: "Keyboard Reference", action: "workspace:show-shortcuts" }],
  },
];
