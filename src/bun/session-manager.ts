import { expandHomePath, formatHomeLabel, resolveShellLaunchConfig } from "./shells";
import type {
  OpenSessionParams,
  SessionBackendInfo,
  SessionExitMessage,
  SessionOutputMessage,
  SessionStartedMessage,
} from "../shared/plexi-rpc";

type TerminalSessionRecord = {
  panelId: string;
  proc: Bun.Subprocess;
  terminal: Bun.Terminal;
  started: SessionStartedMessage;
};

type SessionManagerEvents = {
  onStarted?: (message: SessionStartedMessage) => void;
  onOutput?: (message: SessionOutputMessage) => void;
  onExit?: (message: SessionExitMessage) => void;
  onError?: (panelId: string, error: Error) => void;
};

function isPtySupported() {
  return process.platform !== "win32";
}

export class LocalSessionManager {
  #sessions = new Map<string, TerminalSessionRecord>();
  #events: SessionManagerEvents;
  #env: Record<string, string | undefined>;

  constructor(events: SessionManagerEvents = {}, env: Record<string, string | undefined> = process.env) {
    this.#events = events;
    this.#env = env;
  }

  getBackendInfo(): SessionBackendInfo {
    const config = resolveShellLaunchConfig({ env: this.#env });

    return {
      backend: "bun-pty",
      platform: process.platform,
      supported: isPtySupported(),
      shellPath: config.shellPath,
      shellName: config.shellName,
    };
  }

  async openSession(params: OpenSessionParams) {
    const existing = this.#sessions.get(params.panelId);
    if (existing) {
      return existing.started;
    }

    if (!isPtySupported()) {
      throw new Error("PTY-backed shell sessions are not supported on this platform yet.");
    }

    const launch = resolveShellLaunchConfig({
      cwd: params.cwd,
      env: this.#env,
    });
    const cols = Math.max(20, params.cols || 80);
    const rows = Math.max(8, params.rows || 24);

    const terminal = new Bun.Terminal({
      cols,
      rows,
      data: (_terminal, data) => {
        this.#events.onOutput?.({
          panelId: params.panelId,
          data: Buffer.from(data).toString(),
        });
      },
    });

    const proc = Bun.spawn([launch.shellPath, ...launch.args], {
      cwd: launch.cwd,
      env: launch.env,
      terminal,
    });

    const started: SessionStartedMessage = {
      panelId: params.panelId,
      cwd: launch.cwd,
      cwdLabel: formatHomeLabel(launch.cwd, launch.env.HOME),
      shellPath: launch.shellPath,
      shellName: launch.shellName,
      backend: "bun-pty",
    };

    this.#sessions.set(params.panelId, {
      panelId: params.panelId,
      proc,
      terminal,
      started,
    });

    proc.exited
      .then((exitCode) => {
        this.#sessions.delete(params.panelId);
        this.#events.onExit?.({
          panelId: params.panelId,
          exitCode,
        });
      })
      .catch((error) => {
        this.#sessions.delete(params.panelId);
        this.#events.onError?.(params.panelId, error instanceof Error ? error : new Error(String(error)));
      });

    this.#events.onStarted?.(started);
    return started;
  }

  writeToSession(panelId: string, data: string) {
    const session = this.#sessions.get(panelId);
    if (!session) {
      return;
    }

    session.terminal.write(data);
  }

  resizeSession(panelId: string, cols: number, rows: number) {
    const session = this.#sessions.get(panelId);
    if (!session) {
      return;
    }

    session.terminal.resize(Math.max(20, Math.floor(cols)), Math.max(8, Math.floor(rows)));
  }

  closeSession(panelId: string) {
    const session = this.#sessions.get(panelId);
    if (!session) {
      return;
    }

    this.#sessions.delete(panelId);

    try {
      session.proc.kill();
    } catch (_error) {
      // Ignore double-close races during teardown.
    }

    try {
      session.terminal.close();
    } catch (_error) {
      // Ignore double-close races during teardown.
    }
  }

  closeAllSessions() {
    for (const panelId of this.#sessions.keys()) {
      this.closeSession(panelId);
    }
  }

  resolveCwd(pathname: string | null | undefined) {
    return expandHomePath(pathname, this.#env);
  }
}
