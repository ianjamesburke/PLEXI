import type { ApplicationMenuItemConfig } from "electrobun/bun";

export const QUIT_ACCELERATOR = "CommandOrControl+Q";

export const createApplicationMenu = (): ApplicationMenuItemConfig[] => [
  {
    label: "Plexi",
    submenu: [
      { role: "hide" },
      { role: "hideOthers" },
      { role: "showAll" },
      { type: "separator" },
      { role: "quit", accelerator: QUIT_ACCELERATOR },
    ],
  },
  {
    label: "File",
    submenu: [
      { label: "New Terminal Right", action: "workspace:new-terminal-right", accelerator: "n" },
      { label: "New Terminal Below", action: "workspace:new-terminal-down", accelerator: "N" },
      { type: "separator" },
      { label: "Close Terminal", action: "workspace:close-terminal", accelerator: "w" },
      { label: "Close Window", role: "close", accelerator: "W" },
      { type: "separator" },
      { label: "Save Workspace", action: "workspace:save", accelerator: "s" },
      { label: "Reset Viewport", action: "workspace:reset-viewport", accelerator: "0" },
      { type: "separator" },
      { label: "Quit Plexi", role: "quit", accelerator: QUIT_ACCELERATOR },
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
      { label: "Toggle Overview", action: "workspace:toggle-overview", accelerator: "O" },
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
      { label: "Context 1", action: "workspace:context-1", accelerator: "1" },
      { label: "Context 2", action: "workspace:context-2", accelerator: "2" },
      { label: "Context 3", action: "workspace:context-3", accelerator: "3" },
      { label: "Context 4", action: "workspace:context-4", accelerator: "4" },
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
