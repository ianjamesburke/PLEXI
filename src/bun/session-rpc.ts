import { BrowserView, Utils } from "electrobun/bun";
import type { LocalSessionManager } from "./session-manager";
import type { PlexiRPCSchema } from "../shared/plexi-rpc";
import {
  getWorkspaceFilePath,
  readWorkspaceDocument,
  type WorkspaceStorage,
  writeWorkspaceDocument,
} from "./workspace-file";

export function createSessionRpc(sessionManager: LocalSessionManager, workspaceStorage: WorkspaceStorage) {
  const rpc = BrowserView.defineRPC<PlexiRPCSchema>({
    handlers: {
      requests: {},
      messages: {},
    },
  });

  const requestHandlers = {
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
    getWorkspaceStorageInfo() {
      return {
        path: getWorkspaceFilePath(workspaceStorage),
        source: "disk" as const,
      };
    },
    readWorkspaceDocument() {
      return {
        path: getWorkspaceFilePath(workspaceStorage),
        document: readWorkspaceDocument(workspaceStorage),
      };
    },
    writeWorkspaceDocument(params) {
      writeWorkspaceDocument(workspaceStorage, params.document);
      return {
        path: getWorkspaceFilePath(workspaceStorage),
        source: "disk" as const,
      };
    },
    openExternalUrl(params) {
      return Utils.openExternal(params.url);
    },
    quitApplication() {
      sessionManager.closeAllSessions();
      Utils.quit();
    },
  } as Parameters<typeof rpc.setRequestHandler>[0];

  rpc.setRequestHandler(requestHandlers);

  return rpc;
}
