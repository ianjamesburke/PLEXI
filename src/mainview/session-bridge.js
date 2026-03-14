import { Electroview } from "electrobun/view";
import { createMockSessionBridge } from "./mock-session-bridge.js";

function hasElectrobunRuntime() {
  return (
    typeof window !== "undefined" &&
    typeof window.__electrobunWebviewId === "number" &&
    typeof window.__electrobunRpcSocketPort === "number"
  );
}

function createLiveSessionBridge(handlers) {
  const rpc = Electroview.defineRPC({
    handlers: {
      requests: {},
      messages: {
        sessionStarted(message) {
          handlers.onStarted?.(message);
        },
        sessionOutput(message) {
          handlers.onOutput?.(message);
        },
        sessionExit(message) {
          handlers.onExit?.(message);
        },
        sessionError(message) {
          handlers.onError?.(message);
        },
      },
    },
  });

  new Electroview({ rpc });

  return {
    mode: "live",
    getBackendInfo() {
      return rpc.requestProxy.getBackendInfo();
    },
    openSession(params) {
      return rpc.requestProxy.openSession(params);
    },
    writeToSession(params) {
      return rpc.requestProxy.writeToSession(params);
    },
    resizeSession(params) {
      return rpc.requestProxy.resizeSession(params);
    },
    closeSession(params) {
      return rpc.requestProxy.closeSession(params);
    },
    async reset() {},
  };
}

export function createSessionBridge(handlers = {}) {
  if (hasElectrobunRuntime()) {
    return createLiveSessionBridge(handlers);
  }

  return createMockSessionBridge(handlers);
}
