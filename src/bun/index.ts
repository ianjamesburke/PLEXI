import Electrobun, { ApplicationMenu, BrowserView, BrowserWindow } from "electrobun/bun";
import { LocalSessionManager } from "./session-manager";
import { createApplicationMenu } from "./application-menu";
import type { PlexiRPCSchema } from "../shared/plexi-rpc";

const sessionRpc = BrowserView.defineRPC<PlexiRPCSchema>({
  handlers: {
    requests: {},
    messages: {},
  },
});

const sessionManager = new LocalSessionManager({
  onStarted(message) {
    sessionRpc.sendProxy.sessionStarted(message);
  },
  onOutput(message) {
    sessionRpc.sendProxy.sessionOutput(message);
  },
  onExit(message) {
    sessionRpc.sendProxy.sessionExit(message);
  },
  onError(panelId, error) {
    sessionRpc.sendProxy.sessionError({
      panelId,
      message: error.message,
    });
  },
});

sessionRpc.setRequestHandler({
  getBackendInfo() {
    return sessionManager.getBackendInfo();
  },
  openSession(params) {
    return sessionManager.openSession(params);
  },
  writeToSession(params) {
    sessionManager.writeToSession(params.panelId, params.data);
  },
  resizeSession(params) {
    sessionManager.resizeSession(params.panelId, params.cols, params.rows);
  },
  closeSession(params) {
    sessionManager.closeSession(params.panelId);
  },
});

let mainWindow: BrowserWindow | null = null;

const defaultFrame =
  process.platform === "darwin"
    ? {
      width: 1160,
      height: 720,
      x: 48,
      y: 48,
    }
    : {
      width: 1200,
      height: 800,
      x: 100,
      y: 100,
    };

const dispatchWorkspaceCommand = (command: string) => {
  if (!mainWindow) {
    return;
  }

  const payload = JSON.stringify(command);
  mainWindow.webview.executeJavascript(
    `window.dispatchEvent(new CustomEvent("plexi:command", { detail: { command: ${payload} } }));`,
  );
};

ApplicationMenu.setApplicationMenu(createApplicationMenu());

Electrobun.events.on("application-menu-clicked", (event: unknown) => {
  const action = (event as { data?: { action?: string } })?.data?.action;

  switch (action) {
    case "workspace:new-terminal-right":
      dispatchWorkspaceCommand("new-terminal-right");
      break;
    case "workspace:new-terminal-down":
      dispatchWorkspaceCommand("new-terminal-down");
      break;
    case "workspace:close-terminal":
      dispatchWorkspaceCommand("close-terminal");
      break;
    case "workspace:save":
      dispatchWorkspaceCommand("save-workspace");
      break;
    case "workspace:toggle-overview":
      dispatchWorkspaceCommand("toggle-overview");
      break;
    case "workspace:toggle-sidebar":
      dispatchWorkspaceCommand("toggle-sidebar");
      break;
    case "workspace:reset-viewport":
      dispatchWorkspaceCommand("reset-viewport");
      break;
    case "workspace:zoom-in":
      dispatchWorkspaceCommand("zoom-in");
      break;
    case "workspace:zoom-out":
      dispatchWorkspaceCommand("zoom-out");
      break;
    case "workspace:focus-right":
      dispatchWorkspaceCommand("focus-right");
      break;
    case "workspace:focus-left":
      dispatchWorkspaceCommand("focus-left");
      break;
    case "workspace:focus-up":
      dispatchWorkspaceCommand("focus-up");
      break;
    case "workspace:focus-down":
      dispatchWorkspaceCommand("focus-down");
      break;
    case "workspace:context-1":
      dispatchWorkspaceCommand("context-1");
      break;
    case "workspace:context-2":
      dispatchWorkspaceCommand("context-2");
      break;
    case "workspace:context-3":
      dispatchWorkspaceCommand("context-3");
      break;
    case "workspace:context-4":
      dispatchWorkspaceCommand("context-4");
      break;
    case "workspace:next-context":
      dispatchWorkspaceCommand("next-context");
      break;
    case "workspace:previous-context":
      dispatchWorkspaceCommand("previous-context");
      break;
    case "workspace:show-shortcuts":
      dispatchWorkspaceCommand("show-shortcuts");
      break;
    default:
      break;
  }
});

mainWindow = new BrowserWindow({
  title: "Plexi",
  url: "views://mainview/index.html",
  rpc: sessionRpc,
  titleBarStyle: process.platform === "darwin" ? "hiddenInset" : "default",
  frame: {
    width: defaultFrame.width,
    height: defaultFrame.height,
    x: defaultFrame.x,
    y: defaultFrame.y,
  },
});
