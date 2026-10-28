import { execFile } from "node:child_process";
import type { AttachConfig } from "./attach.ts";

export interface CliResult<T = unknown> {
  ok: boolean;
  exitCode: number;
  stdout: string;
  stderr: string;
  data?: T;
  error?: string;
  errorCode?: string;
}

export type ExecRunner = (
  bin: string,
  args: string[],
  options: { env: NodeJS.ProcessEnv; timeoutMs: number },
) => Promise<{ exitCode: number; stdout: string; stderr: string }>;

const defaultRunner: ExecRunner = (bin, args, options) =>
  new Promise((resolve) => {
    execFile(
      bin,
      args,
      {
        env: options.env,
        timeout: options.timeoutMs,
        maxBuffer: 8 * 1024 * 1024,
      },
      (err, stdout, stderr) => {
        const outStr = typeof stdout === "string" ? stdout : String(stdout ?? "");
        const errStr = typeof stderr === "string" ? stderr : String(stderr ?? "");
        if (!err) {
          resolve({ exitCode: 0, stdout: outStr, stderr: errStr });
          return;
        }
        const code = typeof (err as any).code === "number" ? (err as any).code : 1;
        resolve({
          exitCode: code,
          stdout: outStr,
          stderr: errStr || err.message || "command failed",
        });
      },
    );
  });

/** One resolvable agent, as the orient call reports it: name, tier, harness. */
export interface WorkspaceAgentSummary {
  name: string;
  tier: string;
  harness: string;
}

export interface WorkspaceOverviewSnapshot {
  attached: boolean;
  status: string;
  reason?: string;
  channel: string;
  paneId?: number;
  contextId?: number;
  contextName?: string;
  contexts: any[];
  panes: any[];
  agents: WorkspaceAgentSummary[];
  appTools: any[];
  needsYou: any[];
  continuations: any[];
}

const AGENT_TIER_RANK: Record<string, number> = {
  workspace: 3,
  global: 2,
  "built-in": 1,
};

/**
 * Minimal orient payload from `agent list --json`: the agent array under the
 * top-level `agents` key (unrecognised directories ride beside it under
 * `unrecognised_agent_directories`). name, tier, harness, one row per name. A
 * workspace definition shadows a global one of the same name; the built-in
 * `default` is always present until a workspace definition replaces it.
 */
function summarizeAgents(payload: unknown): WorkspaceAgentSummary[] {
  const rows = (payload as { agents?: unknown } | null)?.agents;
  const byName = new Map<string, WorkspaceAgentSummary>();
  for (const row of Array.isArray(rows) ? rows : []) {
    const name = typeof row?.name === "string" ? row.name : undefined;
    if (!name) continue;
    const tier = typeof row.tier === "string" ? row.tier : "built-in";
    const harness =
      typeof row.harness === "string"
        ? row.harness
        : typeof row.cli === "string"
          ? row.cli
          : "assistant";
    const held = byName.get(name);
    if (
      !held ||
      (AGENT_TIER_RANK[tier] ?? 0) > (AGENT_TIER_RANK[held.tier] ?? 0)
    ) {
      byName.set(name, { name, tier, harness });
    }
  }
  return [...byName.values()].sort((a, b) => a.name.localeCompare(b.name));
}

export class PlexiCliClient {
  private config: AttachConfig;
  private runner: ExecRunner;

  constructor(config: AttachConfig, runner: ExecRunner = defaultRunner) {
    this.config = config;
    this.runner = runner;
  }

  getAttachConfig(): AttachConfig {
    return this.config;
  }

  setAttachConfig(config: AttachConfig): void {
    this.config = config;
  }

  async runRaw(args: string[], timeoutMs = 15_000): Promise<CliResult<string>> {
    if (!this.config.attached) {
      return {
        ok: false,
        exitCode: 1,
        stdout: "",
        stderr: this.config.reason || `Detached (${this.config.status})`,
        error: this.config.reason || `Detached (${this.config.status})`,
        errorCode: this.config.status,
      };
    }

    const env: NodeJS.ProcessEnv = {
      ...process.env,
      ...this.config.env,
      PLEXI_PI_PACKAGE_ACTIVE: "1",
    };

    const res = await this.runner(this.config.plexiBin, args, { env, timeoutMs });
    return {
      ok: res.exitCode === 0,
      exitCode: res.exitCode,
      stdout: res.stdout,
      stderr: res.stderr,
      data: res.stdout,
      error: res.exitCode === 0 ? undefined : (res.stderr.trim() || res.stdout.trim() || `Exit ${res.exitCode}`),
    };
  }

  async runJson<T = any>(args: string[], timeoutMs = 15_000): Promise<CliResult<T>> {
    const raw = await this.runRaw(args, timeoutMs);
    const trimmed = raw.stdout.trim();
    if (!trimmed) {
      return {
        ok: raw.ok,
        exitCode: raw.exitCode,
        stdout: raw.stdout,
        stderr: raw.stderr,
        error: raw.error || "Empty response from plexi CLI",
        errorCode: raw.errorCode,
      };
    }
    try {
      const parsed = JSON.parse(trimmed) as T;
      const obj = parsed as unknown as Record<string, unknown>;
      const explicitOk = typeof obj?.ok === "boolean" ? obj.ok : raw.ok;
      const errorMsg =
        typeof obj?.error === "string"
          ? obj.error
          : !explicitOk
            ? raw.error || "Command returned ok=false"
            : undefined;
      const errorCode =
        typeof obj?.error_code === "string" ? obj.error_code : raw.errorCode;
      return {
        ok: explicitOk,
        exitCode: raw.exitCode,
        stdout: raw.stdout,
        stderr: raw.stderr,
        data: parsed,
        error: errorMsg,
        errorCode,
      };
    } catch (err) {
      return {
        ok: false,
        exitCode: raw.exitCode || 1,
        stdout: raw.stdout,
        stderr: raw.stderr,
        error: `Failed to parse JSON output: ${(err as Error).message}`,
      };
    }
  }

  /** Config lives on disk, so this intentionally works before host attachment. */
  async getModelModes(): Promise<CliResult<any>> {
    const env: NodeJS.ProcessEnv = { ...process.env, ...this.config.env, PLEXI_PI_PACKAGE_ACTIVE: "1" };
    const res = await this.runner(this.config.plexiBin, ["config", "get", "--json", "models"], { env, timeoutMs: 2_000 });
    if (res.exitCode !== 0) return { ok: false, exitCode: res.exitCode, stdout: res.stdout, stderr: res.stderr, error: res.stderr || res.stdout || "config get failed" };
    try { return { ok: true, exitCode: 0, stdout: res.stdout, stderr: res.stderr, data: JSON.parse(res.stdout) }; }
    catch (error) { return { ok: false, exitCode: 1, stdout: res.stdout, stderr: res.stderr, error: `Failed to parse JSON output: ${(error as Error).message}` }; }
  }

  async listPanes(): Promise<CliResult<any[]>> {
    return this.runJson<any[]>(["pane", "list"]);
  }

  async listContexts(): Promise<CliResult<any[]>> {
    return this.runJson<any[]>(["context", "list"]);
  }

  async listAgents(): Promise<CliResult<any[]>> {
    return this.runJson<any[]>(["agent", "list", "--json"]);
  }

  async getPaneInfo(): Promise<CliResult<any>> {
    return this.runJson<any>(["pane", "info"]);
  }

  async getPaneState(paneId: number): Promise<CliResult<any>> {
    return this.runJson<any>(["pane", "state", String(paneId)]);
  }

  async capturePane(paneId: number, lastLines = 80): Promise<CliResult<string>> {
    return this.runRaw(["pane", "capture", "--lines", String(lastLines), String(paneId)]);
  }

  async listAppTools(options: { appId?: string; contextId?: number } = {}): Promise<CliResult<{ ok: boolean; context_id: number; tools: any[] }>> {
    const args = ["app", "tools", "--json"];
    if (options.appId) args.push("--app", options.appId);
    if (options.contextId !== undefined) {
      args.push("--context", String(options.contextId));
    }
    return this.runJson(args);
  }

  async callAppTool(options: {
    appId: string;
    tool: string;
    argsJson?: Record<string, unknown> | string;
    paneId?: number;
    waitApproved?: boolean;
    timeoutMs?: number;
  }): Promise<CliResult<any>> {
    const cliArgs = ["app", "call", options.appId, options.tool, "--json"];
    if (options.argsJson !== undefined) {
      const serialized =
        typeof options.argsJson === "string"
          ? options.argsJson
          : JSON.stringify(options.argsJson);
      cliArgs.push("--input", serialized);
    }
    if (options.paneId !== undefined) {
      cliArgs.push("--pane", String(options.paneId));
    }
    if (options.waitApproved) {
      cliArgs.push("--wait-approved");
    }
    return this.runJson(cliArgs, options.timeoutMs ?? (options.waitApproved ? 65_000 : 20_000));
  }

  async listNeedsYou(): Promise<CliResult<{ ok: boolean; items: any[] }>> {
    return this.runJson(["needs-you", "list", "--json"]);
  }

  async showNeedsYou(id: string): Promise<CliResult<{ ok: boolean; id: string; item?: any; continuation?: any }>> {
    return this.runJson(["needs-you", "show", id]);
  }

  async askNeedsYou(options: {
    summary: string;
    resource?: string;
    choices?: string[];
    allowFreeText?: boolean;
    previewRefs?: string[];
    correlationId?: string;
    operationId?: string;
    sessionId?: string;
    expectedRevision?: string;
    contextId?: number;
    paneId?: number;
  }): Promise<CliResult<{ ok: boolean; id: string; ui_token?: string; item?: any; continuation?: any }>> {
    const args = ["needs-you", "ask", "--summary", options.summary];
    if (options.resource) {
      args.push("--resource", options.resource);
    }
    for (const c of options.choices ?? []) {
      args.push("--choice", c);
    }
    if (options.allowFreeText) {
      args.push("--allow-free-text");
    }
    for (const ref of options.previewRefs ?? []) {
      args.push("--preview-ref", ref);
    }
    if (options.correlationId) {
      args.push("--correlation-id", options.correlationId);
    }
    if (options.operationId) {
      args.push("--operation-id", options.operationId);
    }
    if (options.sessionId) {
      args.push("--session-id", options.sessionId);
    }
    if (options.expectedRevision) {
      args.push("--expected-revision", options.expectedRevision);
    }
    if (options.contextId !== undefined) {
      args.push("--context-id", String(options.contextId));
    }
    if (options.paneId !== undefined) {
      args.push("--pane-id", String(options.paneId));
    }
    return this.runJson(args);
  }

  async answerNeedsYou(options: {
    id: string;
    answer: string;
    uiToken?: string;
  }): Promise<CliResult<any>> {
    const args = ["needs-you", "answer", options.id, "--answer", options.answer];
    if (options.uiToken) {
      args.push("--ui-token", options.uiToken);
    }
    return this.runJson(args);
  }

  async reconcileNeedsYou(options: {
    sessionId?: string;
    paneId?: number;
    contextId?: number;
    id?: string;
  } = {}): Promise<CliResult<{ ok: boolean; items: any[]; continuations: any[] }>> {
    const args = ["needs-you", "reconcile"];
    if (options.sessionId) args.push("--session-id", options.sessionId);
    if (options.paneId !== undefined) args.push("--pane-id", String(options.paneId));
    if (options.contextId !== undefined) args.push("--context-id", String(options.contextId));
    if (options.id) args.push("--id", options.id);
    return this.runJson(args);
  }

  async completeContinuation(options: {
    id: string;
    state?: "completed" | "failed" | "canceled" | "outcome_unknown";
    receipt?: string;
  }): Promise<CliResult<{ ok: boolean; continuation: any }>> {
    const args = ["needs-you", "complete", options.id];
    if (options.state) args.push("--state", options.state);
    if (options.receipt) args.push("--receipt", options.receipt);
    return this.runJson(args);
  }

  async reportAgentState(options: {
    state: "working" | "blocked" | "idle";
    detail?: string;
    sessionId?: string;
    blockedReason?: "permission_prompt" | "question" | "usage_limit" | "error";
  }): Promise<CliResult<string>> {
    const args = ["agent", "report", "--state", options.state, "--agent", "pi"];
    if (options.detail) args.push("--detail", options.detail);
    if (options.sessionId) args.push("--session-id", options.sessionId);
    if (options.blockedReason) {
      const mapped =
        options.blockedReason === "permission_prompt" || options.blockedReason === "question"
          ? "permission-prompt"
          : options.blockedReason === "usage_limit"
            ? "usage-limit"
            : "unknown";
      args.push("--blocked-reason", mapped);
    }
    return this.runRaw(args, 5_000);
  }

  async takeScreenshot(options: {
    outputPath: string;
    paneId?: number;
  }): Promise<CliResult<string>> {
    const args = ["host", "screenshot", "--output", options.outputPath];
    if (options.paneId !== undefined) {
      args.push("--pane", String(options.paneId));
    }
    return this.runRaw(args, 20_000);
  }

  async listCommands(): Promise<
    CliResult<
      Array<{
        name: string;
        scope: "workspace" | "global";
        description: string | null;
        run: string | null;
        secrets: string[];
      }>
    >
  > {
    return this.runJson(["run", "--json"]);
  }

  async runCommand(options: {
    command: string;
    args?: string[];
    timeoutMs?: number;
  }): Promise<CliResult<string>> {
    const cliArgs = ["run", options.command];
    if (options.args && options.args.length > 0) {
      cliArgs.push("--", ...options.args);
    }
    return this.runRaw(cliArgs, options.timeoutMs ?? 30_000);
  }

  async fetchWorkspaceOverview(sessionId?: string): Promise<WorkspaceOverviewSnapshot> {
    if (!this.config.attached) {
      return {
        attached: false,
        status: this.config.status,
        reason: this.config.reason,
        channel: this.config.channel,
        paneId: this.config.paneId,
        contextId: this.config.contextId,
        contexts: [],
        panes: [],
        agents: [],
        appTools: [],
        needsYou: [],
        continuations: [],
      };
    }

    const [panesRes, contextsRes, agentsRes, toolsRes, reconcileRes] =
      await Promise.all([
        this.listPanes(),
        this.listContexts(),
        this.listAgents(),
        this.listAppTools({ contextId: this.config.contextId }),
        this.reconcileNeedsYou({
          sessionId,
          paneId: this.config.paneId,
          contextId: this.config.contextId,
        }),
      ]);

    const allPanes = Array.isArray(panesRes.data) ? panesRes.data : [];
    const contexts = Array.isArray(contextsRes.data) ? contextsRes.data : [];
    const selfPane = allPanes.find((p) => Number(p.id) === this.config.paneId);
    const activeCtxId =
      selfPane?.context_id ??
      this.config.contextId ??
      contexts.find((c) => c.is_active)?.context_id;
    const contextPanes =
      activeCtxId !== undefined
        ? allPanes.filter((p) => Number(p.context_id) === Number(activeCtxId))
        : allPanes;
    const contextObj = contexts.find((c) => Number(c.context_id) === Number(activeCtxId));

    return {
      attached: true,
      status: this.config.status,
      channel: this.config.channel,
      paneId: this.config.paneId,
      contextId: activeCtxId,
      contextName: selfPane?.context_name ?? contextObj?.name,
      contexts,
      panes: contextPanes,
      agents: summarizeAgents(agentsRes.data),
      appTools: Array.isArray(toolsRes.data?.tools) ? toolsRes.data.tools : [],
      needsYou: Array.isArray(reconcileRes.data?.items)
        ? reconcileRes.data.items.filter((i: any) => !i.resolution)
        : [],
      continuations: Array.isArray(reconcileRes.data?.continuations)
        ? reconcileRes.data.continuations
        : [],
    };
  }
}
