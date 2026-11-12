import { existsSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";

export type AttachStatus =
  | "attached"
  | "detached_outside_plexi"
  | "detached_socket_missing"
  | "detached_channel_mismatch"
  | "detached_stale_identity"
  | "detached_host_unreachable";

export interface AttachConfig {
  status: AttachStatus;
  attached: boolean;
  reason?: string;
  socketPath?: string;
  paneId?: number;
  contextId?: number;
  contextRoot?: string;
  channel: string;
  plexiBin: string;
  env: Record<string, string | undefined>;
}

/**
 * Derive the channel name (`stable`, `alpha`, `beta`, `pr-123`, `local-...`) from
 * a socket path such as `/Users/u/.plexi-alpha/plexi.sock`.
 */
export function inferChannelFromSocketPath(socketPath: string | undefined): string | undefined {
  if (!socketPath) return undefined;
  const normalized = resolve(socketPath);
  const parts = normalized.split(/[\\/]/);
  for (const part of parts) {
    if (part === ".plexi") return "stable";
    if (part.startsWith(".plexi-")) {
      const suffix = part.slice(".plexi-".length).trim();
      if (suffix) return suffix;
    }
  }
  return undefined;
}

/**
 * Resolve the Plexi CLI binary to execute, honoring `PLEXI_CLI_BIN` if explicitly
 * provided by a test harness or channel wrapper, otherwise falling back to `plexi`.
 */
export function resolvePlexiBinary(env: Record<string, string | undefined> = process.env): string {
  const explicit = env.PLEXI_CLI_BIN?.trim();
  if (explicit) return explicit;
  return "plexi";
}

/**
 * Inspect ambient environment variables and filesystem socket state to determine
 * whether this Pi session is attached to a live Plexi pane.
 */
export function detectAttachment(
  env: Record<string, string | undefined> = process.env,
  options: { checkSocketExists?: (path: string) => boolean } = {},
): AttachConfig {
  const explicitSocket = env.PLEXI_SOCKET?.trim();
  const rawPaneId = env.PLEXI_PANE_ID?.trim();
  const rawContextId = env.PLEXI_CONTEXT_ID?.trim();
  const contextRoot = env.PLEXI_CONTEXT_ROOT?.trim();
  const explicitChannel = env.PLEXI_CHANNEL?.trim();
  const socketChannel = inferChannelFromSocketPath(explicitSocket);
  const channel = explicitChannel || socketChannel || "stable";
  const plexiBin = resolvePlexiBinary(env);

  if (!explicitSocket && !rawPaneId) {
    return {
      status: "detached_outside_plexi",
      attached: false,
      reason: "Running outside a Plexi terminal pane (PLEXI_SOCKET and PLEXI_PANE_ID are unset).",
      channel,
      plexiBin,
      env,
    };
  }

  let socketPath = explicitSocket;
  if (!socketPath && rawPaneId && explicitChannel) {
    const home = env.HOME?.trim() || homedir();
    const profileDir =
      explicitChannel === "stable" ? join(home, ".plexi") : join(home, `.plexi-${explicitChannel}`);
    socketPath = join(profileDir, "notify.sock");
  }

  if (!socketPath) {
    return {
      status: "detached_stale_identity",
      attached: false,
      reason: "PLEXI_PANE_ID is set without PLEXI_SOCKET.",
      channel,
      plexiBin,
      env,
    };
  }

  let paneId: number | undefined;
  if (rawPaneId !== undefined && rawPaneId !== "") {
    const parsed = Number(rawPaneId);
    if (!Number.isInteger(parsed) || parsed < 0) {
      return {
        status: "detached_stale_identity",
        attached: false,
        reason: `Invalid PLEXI_PANE_ID '${rawPaneId}'.`,
        socketPath,
        channel,
        plexiBin,
        env,
      };
    }
    paneId = parsed;
  } else {
    return {
      status: "detached_stale_identity",
      attached: false,
      reason: "PLEXI_SOCKET is set without PLEXI_PANE_ID.",
      socketPath,
      channel,
      plexiBin,
      env,
    };
  }

  let contextId: number | undefined;
  if (rawContextId !== undefined && rawContextId !== "") {
    const parsedCtx = Number(rawContextId);
    if (!Number.isInteger(parsedCtx) || parsedCtx < 0) {
      return {
        status: "detached_stale_identity",
        attached: false,
        reason: `Invalid PLEXI_CONTEXT_ID '${rawContextId}'.`,
        socketPath,
        paneId,
        channel,
        plexiBin,
        env,
      };
    }
    contextId = parsedCtx;
  }

  if (
    explicitChannel &&
    socketChannel &&
    explicitChannel !== socketChannel &&
    !(explicitChannel === "stable" && socketChannel === "stable")
  ) {
    return {
      status: "detached_channel_mismatch",
      attached: false,
      reason: `Channel mismatch: PLEXI_CHANNEL='${explicitChannel}' does not match socket profile '${socketChannel}' (${socketPath}).`,
      socketPath,
      paneId,
      contextId,
      contextRoot,
      channel: explicitChannel,
      plexiBin,
      env,
    };
  }

  const checkExists =
    options.checkSocketExists ??
    ((p: string) => {
      try {
        if (!existsSync(p)) return false;
        const st = statSync(p);
        return st.isSocket() || st.isFile();
      } catch {
        return false;
      }
    });

  if (!checkExists(socketPath)) {
    return {
      status: "detached_socket_missing",
      attached: false,
      reason: `Plexi host socket does not exist at ${socketPath} (host may have stopped or restarted).`,
      socketPath,
      paneId,
      contextId,
      contextRoot,
      channel,
      plexiBin,
      env,
    };
  }

  return {
    status: "attached",
    attached: true,
    socketPath,
    paneId,
    contextId,
    contextRoot,
    channel,
    plexiBin,
    env: {
      ...env,
      PLEXI_SOCKET: socketPath,
    },
  };
}

/**
 * Verify that the live host actually recognizes this pane and context identity.
 * If the pane was closed or belongs to an earlier host run, returns a degraded
 * `AttachConfig` with `detached_stale_identity` or `detached_host_unreachable`.
 */
export function verifyLiveIdentity(
  config: AttachConfig,
  panes: Array<{ id?: number; context_id?: number }> | undefined,
  error?: string,
): AttachConfig {
  if (!config.attached) return config;
  if (error || !Array.isArray(panes)) {
    return {
      ...config,
      status: "detached_host_unreachable",
      attached: false,
      reason: error || "Could not query live panes from Plexi host.",
    };
  }
  const found = panes.find((p) => Number(p.id) === config.paneId);
  if (!found) {
    return {
      ...config,
      status: "detached_stale_identity",
      attached: false,
      reason: `Pane ${config.paneId} is not present in the live host (${basename(dirname(config.socketPath || ""))}).`,
    };
  }
  if (
    config.contextId !== undefined &&
    found.context_id !== undefined &&
    Number(found.context_id) !== config.contextId
  ) {
    return {
      ...config,
      contextId: Number(found.context_id),
    };
  }
  return config;
}
