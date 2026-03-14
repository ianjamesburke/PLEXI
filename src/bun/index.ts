import Electrobun, { ApplicationMenu } from "electrobun/bun";
import { createMainWindow } from "./main-window";
import { LocalSessionManager } from "./session-manager";
import { createApplicationMenu } from "./application-menu";
import { createSessionRpc } from "./session-rpc";
import { dispatchWorkspaceCommand, getWorkspaceCommandForMenuAction } from "./workspace-commands";
import { resolveLaunchOptions } from "./launch-options";
import { createWorkspaceStorage, resetWorkspaceStorage } from "./workspace-file";

const launchOptions = resolveLaunchOptions();
const workspaceStorage = createWorkspaceStorage({
  profile: launchOptions.profile,
});

if (launchOptions.clean) {
  resetWorkspaceStorage(workspaceStorage);
}

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
const sessionRpc = createSessionRpc(sessionManager, workspaceStorage);
let mainWindow = createMainWindow(sessionRpc);
const applicationMenu = createApplicationMenu();

function installApplicationMenu() {
  ApplicationMenu.setApplicationMenu(applicationMenu);
}

installApplicationMenu();

if (process.platform === "darwin") {
  // Electrobun appears to occasionally miss the first install during app activation.
  setTimeout(installApplicationMenu, 0);
  setTimeout(installApplicationMenu, 200);
  mainWindow.on("focus", installApplicationMenu);
}

ApplicationMenu.on("application-menu-clicked", (event: unknown) => {
  const action = (event as { data?: { action?: string } })?.data?.action;
  dispatchWorkspaceCommand(mainWindow, getWorkspaceCommandForMenuAction(action));
});

for (const signal of ["exit", "SIGINT", "SIGTERM"] as const) {
  process.on(signal, () => {
    sessionManager.closeAllSessions();
  });
}
