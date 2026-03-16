import { expandHomePath, formatHomeLabel, resolveShellLaunchConfig } from "./shells";
import type { IDisposable, IPty } from "bun-pty";
import type {
  OpenSessionParams,
  SessionBackendInfo,
  SessionExitMessage,
  SessionOutputMessage,
  SessionStartedMessage,
} from "../shared/plexi-rpc";
import { loadBunPty } from "./pty-loader";
import { applyShellBootstrap, cleanupBootstrapDir } from "./shell-bootstrap";

type TerminalSessionRecord = {
  panelId: string;
  pty: IPty;
  outputSubscription: IDisposable;
  exitSubscription: IDisposable;
  started: SessionStartedMessage;
  bootstrapDir?: string;
};

type SessionManagerEvents = {
  onStarted?: (message: SessionStartedMessage) => void;
  onOutput?: (message: SessionOutputMessage) => void;
  onExit?: (message: SessionExitMessage) => void;
  onError?: (panelId: string, error: Error) => void;
};

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
      supported: true,
      shellPath: config.shellPath,
      shellName: config.shellName,
    };
  }

  async openSession(params: OpenSessionParams) {
    const existing = this.#sessions.get(params.panelId);
    if (existing) {
      return existing.started;
    }

    const { launch: bootstrappedLaunch, bootstrapDir } = applyShellBootstrap(resolveShellLaunchConfig({
      cwd: params.cwd,
      env: this.#env,
    }));
    const cols = Math.max(20, params.cols || 80);
    const rows = Math.max(8, params.rows || 24);
    const { spawn } = await loadBunPty();

    const pty = spawn(bootstrappedLaunch.shellPath, bootstrappedLaunch.args, {
      name: bootstrappedLaunch.env["TERM"] || "xterm-256color",
      cols,
      rows,
      cwd: bootstrappedLaunch.cwd,
      env: bootstrappedLaunch.env,
    });

    const outputSubscription = pty.onData((data) => {
        this.#events.onOutput?.({
          panelId: params.panelId,
          data,
        });
    });
    const exitSubscription = pty.onExit((event) => {
      const record = this.#sessions.get(params.panelId);
      this.#sessions.delete(params.panelId);
      cleanupBootstrapDir(record?.bootstrapDir);
      this.#events.onExit?.({
        panelId: params.panelId,
        exitCode: event.exitCode,
      });
    });

    const started: SessionStartedMessage = {
      panelId: params.panelId,
      cwd: bootstrappedLaunch.cwd,
      cwdLabel: formatHomeLabel(bootstrappedLaunch.cwd, bootstrappedLaunch.env["HOME"]),
      shellPath: bootstrappedLaunch.shellPath,
      shellName: bootstrappedLaunch.shellName,
      backend: "bun-pty",
    };

    this.#sessions.set(params.panelId, {
      panelId: params.panelId,
      pty,
      outputSubscription,
      exitSubscription,
      started,
      bootstrapDir: bootstrapDir || undefined,
    });

    this.#events.onStarted?.(started);
    return started;
  }

  writeToSession(panelId: string, data: string) {
    const session = this.#sessions.get(panelId);
    if (!session) {
      return;
    }

    try {
      session.pty.write(data);
    } catch (error) {
      this.#events.onError?.(panelId, error instanceof Error ? error : new Error(String(error)));
    }
  }

  resizeSession(panelId: string, cols: number, rows: number) {
    const session = this.#sessions.get(panelId);
    if (!session) {
      return;
    }

    try {
      session.pty.resize(Math.max(20, Math.floor(cols)), Math.max(8, Math.floor(rows)));
    } catch (_error) {
      // Ignore resize races when the PTY process has already exited.
    }
  }

  closeSession(panelId: string) {
    const session = this.#sessions.get(panelId);
    if (!session) {
      return;
    }

    this.#sessions.delete(panelId);
    session.outputSubscription.dispose();
    session.exitSubscription.dispose();

    try {
      session.pty.kill();
    } catch (_error) {
      // Ignore double-close races during teardown.
    }
    cleanupBootstrapDir(session.bootstrapDir);
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
