import { spawn, type ChildProcess } from "node:child_process";
import { createInterface } from "node:readline";
import type { AttachConfig } from "./attach.ts";

export interface PlexiEventRecord {
  seq?: number;
  timestamp?: number;
  context_id?: number;
  app_id?: string;
  instance_id?: number;
  event: string;
  actor?: string;
  summary: string;
  resource_id?: string;
  resource_scope?: string;
  revision_before?: string;
  revision_after?: string;
  payload?: unknown;
}

export class PlexiEventSubscriber {
  private config: AttachConfig;
  private maxBuffer: number;
  private buffer: PlexiEventRecord[] = [];
  private children = new Map<string, ChildProcess>();
  private onEventCallback?: (ev: PlexiEventRecord) => void;

  constructor(
    config: AttachConfig,
    options: {
      maxBuffer?: number;
      onEvent?: (ev: PlexiEventRecord) => void;
    } = {},
  ) {
    this.config = config;
    this.maxBuffer = options.maxBuffer ?? 50;
    this.onEventCallback = options.onEvent;
  }

  setAttachConfig(config: AttachConfig): void {
    this.config = config;
  }

  ingestLine(line: string): PlexiEventRecord | undefined {
    const trimmed = line.trim();
    if (!trimmed) return undefined;
    try {
      const parsed = JSON.parse(trimmed) as PlexiEventRecord;
      if (!parsed || typeof parsed !== "object" || typeof parsed.event !== "string") {
        return undefined;
      }
      this.pushEvent(parsed);
      return parsed;
    } catch {
      return undefined;
    }
  }

  pushEvent(ev: PlexiEventRecord): void {
    this.buffer.push(ev);
    if (this.buffer.length > this.maxBuffer) {
      this.buffer.splice(0, this.buffer.length - this.maxBuffer);
    }
    this.onEventCallback?.(ev);
  }

  getRecentEvents(limit = 10, filter?: { appId?: string; stream?: string }): PlexiEventRecord[] {
    const filtered = this.buffer.filter((ev) => {
      if (filter?.appId && ev.app_id && ev.app_id !== filter.appId) return false;
      if (filter?.stream && ev.event !== filter.stream) return false;
      return true;
    });
    return filtered.slice(-Math.max(1, limit));
  }

  activeSubscriptions(): string[] {
    return Array.from(this.children.keys());
  }

  subscribe(appId: string, stream: string, options: { paneId?: number; resource?: string } = {}): { ok: boolean; key: string; reason?: string } {
    const key = `${appId}:${stream}`;
    if (!this.config.attached) {
      return { ok: false, key, reason: this.config.reason || "Detached from Plexi host" };
    }
    if (this.children.has(key)) {
      return { ok: true, key };
    }
    const args = ["events", "subscribe", appId, stream];
    if (options.resource) {
      args.push("--resource", options.resource);
    }
    try {
      const child = spawn(this.config.plexiBin, args, {
        env: {
          ...process.env,
          ...this.config.env,
          PLEXI_PI_PACKAGE_ACTIVE: "1",
        },
        stdio: ["ignore", "pipe", "pipe"],
      });
      this.children.set(key, child);

      if (child.stdout) {
        const rl = createInterface({ input: child.stdout });
        rl.on("line", (line) => {
          this.ingestLine(line);
        });
      }
      child.on("exit", () => {
        this.children.delete(key);
      });
      child.on("error", () => {
        this.children.delete(key);
      });
      return { ok: true, key };
    } catch (err) {
      return { ok: false, key, reason: (err as Error).message };
    }
  }

  stopAll(): void {
    for (const [key, child] of this.children.entries()) {
      try {
        child.kill("SIGTERM");
      } catch {
        // ignore
      }
      this.children.delete(key);
    }
  }
}
