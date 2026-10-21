import { tmpdir } from "node:os";
import { join } from "node:path";
import type { PlexiCliClient } from "./cli-client.ts";
import type { PlexiEventSubscriber } from "./events.ts";
import { resolveSessionIdFromContext, type PlexiLifecycleReporter } from "./lifecycle.ts";
import { applyWorkspaceUi } from "./ui.ts";

export interface ToolResponse {
  content: Array<{ type: "text"; text: string }>;
  details?: unknown;
  isError?: boolean;
}

function textResult(payload: unknown, isError = false): ToolResponse {
  const text =
    typeof payload === "string" ? payload : JSON.stringify(payload, null, 2);
  return {
    content: [{ type: "text", text }],
    details: payload,
    isError,
  };
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export interface PlexiToolDeps {
  client: PlexiCliClient;
  events: PlexiEventSubscriber;
  lifecycle: PlexiLifecycleReporter;
  onUiRefresh?: (ctx: any) => Promise<void>;
}

export function buildPlexiTools(deps: PlexiToolDeps) {
  const { client, events, lifecycle, onUiRefresh } = deps;

  const workspaceOverviewTool = {
    name: "plexi_workspace_overview",
    label: "Plexi Workspace Overview",
    description:
      "Inspect the current Plexi workspace: attachment status, active context, sibling terminal and app panes, live app tools, open Needs you items, and durable continuations.",
    parameters: {
      type: "object",
      properties: {
        session_id: {
          type: "string",
          description: "Optional session id override for reconciling continuations.",
        },
      },
      additionalProperties: false,
    },
    async execute(_toolCallId: string, params: Record<string, any>, _signal: AbortSignal | undefined, _onUpdate: any, ctx: any): Promise<ToolResponse> {
      const sid = params?.session_id || resolveSessionIdFromContext(ctx);
      const snapshot = await client.fetchWorkspaceOverview(sid);
      applyWorkspaceUi(ctx, snapshot, events.getRecentEvents(5));
      return textResult(
        {
          ...snapshot,
          recentEvents: events.getRecentEvents(10),
        },
        !snapshot.attached,
      );
    },
  };

  const paneInspectTool = {
    name: "plexi_pane_inspect",
    label: "Plexi Pane Inspect",
    description:
      "Inspect a Plexi terminal or app pane by pane_id: reads structured pane info/state (`plexi pane state <id> --json`) and optionally captures terminal output lines (`plexi pane capture <id>`).",
    parameters: {
      type: "object",
      properties: {
        pane_id: {
          type: "number",
          description: "Target pane ID (defaults to current pane if omitted).",
        },
        capture: {
          type: "boolean",
          description: "If true, also capture terminal text lines from the pane.",
        },
        last_lines: {
          type: "number",
          description: "Number of trailing terminal lines to capture (default 60).",
        },
      },
      additionalProperties: false,
    },
    async execute(_toolCallId: string, params: Record<string, any>, _signal: AbortSignal | undefined, _onUpdate: any, _ctx: any): Promise<ToolResponse> {
      const cfg = client.getAttachConfig();
      if (!cfg.attached) {
        return textResult({ ok: false, error: cfg.reason, status: cfg.status }, true);
      }
      const targetPaneId =
        typeof params?.pane_id === "number" ? params.pane_id : cfg.paneId!;
      const stateRes = await client.getPaneState(targetPaneId);
      let capturedText: string | undefined;
      if (params?.capture) {
        const capRes = await client.capturePane(
          targetPaneId,
          typeof params?.last_lines === "number" ? params.last_lines : 60,
        );
        if (capRes.ok) {
          capturedText = capRes.stdout;
        }
      }
      return textResult(
        {
          ok: stateRes.ok,
          pane_id: targetPaneId,
          state: stateRes.data,
          capture: capturedText,
          error: stateRes.error,
        },
        !stateRes.ok,
      );
    },
  };

  const appToolsTool = {
    name: "plexi_app_tools",
    label: "Plexi App Tools",
    description:
      "Discover live app tools and their JSON input/output schemas in the current Plexi context (`plexi app tools --json`).",
    parameters: {
      type: "object",
      properties: {
        app_id: {
          type: "string",
          description: "Optional app ID filter (e.g. 'clip-editor-probe' or 'todo').",
        },
        context_id: {
          type: "number",
          description: "Optional context ID override.",
        },
      },
      additionalProperties: false,
    },
    async execute(_toolCallId: string, params: Record<string, any>, _signal: AbortSignal | undefined, _onUpdate: any, _ctx: any): Promise<ToolResponse> {
      const res = await client.listAppTools({
        appId: typeof params?.app_id === "string" ? params.app_id : undefined,
        contextId: typeof params?.context_id === "number" ? params.context_id : undefined,
      });
      return textResult(res.data ?? { ok: false, error: res.error }, !res.ok);
    },
  };

  const appCallTool = {
    name: "plexi_app_call",
    label: "Plexi App Call",
    description:
      "Invoke an app-exposed tool (`plexi app call <app> <tool> --args <json>`). Read-only tools run immediately. Mutating tools (`read_only=false`) are gated on the exact target resource and revision; when `wait_approved` is true, waits for the human to click Allow once in Plexi.",
    parameters: {
      type: "object",
      properties: {
        app_id: {
          type: "string",
          description: "Target app ID (e.g. 'clip-editor-probe' or 'todo').",
        },
        tool: {
          type: "string",
          description: "Tool name (e.g. 'clip.state', 'clip.crop', 'clip.move').",
        },
        args: {
          type: "object",
          description: "JSON arguments object for the tool.",
        },
        pane_id: {
          type: "number",
          description: "Optional app instance pane ID when multiple instances exist.",
        },
        wait_approved: {
          type: "boolean",
          description: "If true, wait for the human to approve an ask-tier tool in the Plexi desktop window.",
        },
        complete_continuation_id: {
          type: "string",
          description: "Optional continuation run/operation/correlation ID to mark completed when the tool call succeeds.",
        },
      },
      required: ["app_id", "tool"],
      additionalProperties: false,
    },
    async execute(_toolCallId: string, params: Record<string, any>, _signal: AbortSignal | undefined, _onUpdate: any, ctx: any): Promise<ToolResponse> {
      const sid = resolveSessionIdFromContext(ctx);
      const waitApproved = Boolean(params?.wait_approved);
      if (waitApproved) {
        await lifecycle.report("blocked", {
          detail: `Waiting approval: ${params.app_id}.${params.tool}`,
          sessionId: sid,
          blockedReason: "permission_prompt",
        });
      }
      const res = await client.callAppTool({
        appId: String(params.app_id),
        tool: String(params.tool),
        argsJson: params?.args ?? {},
        paneId: typeof params?.pane_id === "number" ? params.pane_id : undefined,
        waitApproved,
      });

      const outObj = res.data as Record<string, any> | undefined;
      const innerOutput = outObj?.output as Record<string, any> | undefined;
      const innerError =
        innerOutput && typeof innerOutput === "object" && innerOutput.ok === false;

      if (res.ok && !innerError) {
        const contId =
          params?.complete_continuation_id ||
          (typeof params?.args?.operation_id === "string" ? params.args.operation_id : undefined);
        if (contId) {
          await client.completeContinuation({
            id: String(contId),
            state: "completed",
            receipt: JSON.stringify(innerOutput ?? outObj ?? { ok: true }),
          });
        }
      } else if (innerError && innerOutput?.error_code === "stale_revision") {
        const contId =
          params?.complete_continuation_id ||
          (typeof params?.args?.operation_id === "string" ? params.args.operation_id : undefined);
        if (contId) {
          await client.completeContinuation({
            id: String(contId),
            state: "failed",
            receipt: JSON.stringify(innerOutput),
          });
        }
      }

      if (onUiRefresh) {
        await onUiRefresh(ctx);
      }

      const isErr = !res.ok || Boolean(innerError);
      return textResult(
        res.data ?? {
          ok: false,
          error: res.error,
          error_code: res.errorCode,
        },
        isErr,
      );
    },
  };

  const eventsPollTool = {
    name: "plexi_events_poll",
    label: "Plexi Events Poll",
    description:
      "Subscribe to or poll recent app events from `plexi events subscribe <app> <stream>`.",
    parameters: {
      type: "object",
      properties: {
        app_id: {
          type: "string",
          description: "Optional app ID to subscribe to or filter by (e.g. 'clip-editor-probe').",
        },
        stream: {
          type: "string",
          description: "Optional event stream name to subscribe to or filter by (e.g. 'clip.updated').",
        },
        subscribe: {
          type: "boolean",
          description: "If true, ensure a live background subscription is running for app_id + stream.",
        },
        limit: {
          type: "number",
          description: "Maximum number of buffered events to return (default 10).",
        },
      },
      additionalProperties: false,
    },
    async execute(_toolCallId: string, params: Record<string, any>, _signal: AbortSignal | undefined, _onUpdate: any, _ctx: any): Promise<ToolResponse> {
      let subStatus: { ok: boolean; key: string; reason?: string } | undefined;
      if (params?.subscribe && params?.app_id && params?.stream) {
        subStatus = events.subscribe(String(params.app_id), String(params.stream));
      }
      const recent = events.getRecentEvents(
        typeof params?.limit === "number" ? params.limit : 10,
        {
          appId: typeof params?.app_id === "string" ? params.app_id : undefined,
          stream: typeof params?.stream === "string" ? params.stream : undefined,
        },
      );
      return textResult({
        ok: true,
        active_subscriptions: events.activeSubscriptions(),
        subscription: subStatus,
        events: recent,
      });
    },
  };

  const needsYouAskTool = {
    name: "plexi_needs_you_ask",
    label: "Plexi Needs You Ask",
    description:
      "File a structured creative or blocking question for the human (`choices`, `allow_free_text`, `preview_refs`, `correlation_id`, `expected_revision`). Appears in both Pi and the Plexi host's Needs you surface. Answering a question never mints a permission grant.",
    parameters: {
      type: "object",
      properties: {
        summary: {
          type: "string",
          description: "Question prompt shown to the human.",
        },
        resource: {
          type: "string",
          description: "Target resource ID (e.g. clip_id 'intro').",
        },
        choices: {
          type: "array",
          items: { type: "string" },
          description: "Selectable choices (e.g. ['16:9', '9:16', '1:1']).",
        },
        allow_free_text: {
          type: "boolean",
          description: "Whether the human can enter custom free text outside choices.",
        },
        preview_refs: {
          type: "array",
          items: { type: "string" },
          description: "Preview references (e.g. ['clip:intro@0..3000ms']).",
        },
        correlation_id: {
          type: "string",
          description: "Correlation ID joining this question to a continuation record.",
        },
        operation_id: {
          type: "string",
          description: "Operation ID for idempotent resume after the human answers.",
        },
        expected_revision: {
          type: "string",
          description: "Expected resource revision (e.g. 'rev-1') to verify when resuming.",
        },
        wait_for_answer: {
          type: "boolean",
          description: "If true, wait for the human to answer (via Pi TUI dialog or Plexi desktop surface) before returning.",
        },
        timeout_ms: {
          type: "number",
          description: "Maximum time in ms to wait when wait_for_answer=true (default 30000).",
        },
        prompt_in_pi_ui: {
          type: "boolean",
          description: "If true and interactive Pi UI is active, also present a local TUI selector when waiting.",
        },
      },
      required: ["summary"],
      additionalProperties: false,
    },
    async execute(_toolCallId: string, params: Record<string, any>, signal: AbortSignal | undefined, _onUpdate: any, ctx: any): Promise<ToolResponse> {
      const sid = resolveSessionIdFromContext(ctx);
      const choices: string[] = Array.isArray(params?.choices)
        ? params.choices.map(String)
        : [];
      const previewRefs: string[] = Array.isArray(params?.preview_refs)
        ? params.preview_refs.map(String)
        : [];
      const askRes = await client.askNeedsYou({
        summary: String(params.summary),
        resource: typeof params?.resource === "string" ? params.resource : "workspace",
        choices,
        allowFreeText: Boolean(params?.allow_free_text),
        previewRefs,
        correlationId:
          typeof params?.correlation_id === "string" ? params.correlation_id : undefined,
        operationId:
          typeof params?.operation_id === "string" ? params.operation_id : undefined,
        sessionId: sid,
        expectedRevision:
          typeof params?.expected_revision === "string" ? params.expected_revision : undefined,
      });

      if (!askRes.ok || !askRes.data?.id) {
        return textResult(
          askRes.data ?? { ok: false, error: askRes.error },
          true,
        );
      }

      const itemId = askRes.data.id;
      const uiToken = askRes.data.ui_token;

      await lifecycle.report("blocked", {
        detail: `Question: ${params.summary}`,
        sessionId: sid,
        blockedReason: "question",
      });

      if (onUiRefresh) {
        await onUiRefresh(ctx);
      }

      if (!params?.wait_for_answer) {
        return textResult({
          ok: true,
          status: "waiting_user",
          id: itemId,
          item: askRes.data.item,
          continuation: askRes.data.continuation,
        });
      }

      // If interactive Pi UI selector is requested and available, let the human answer in Pi UI
      // using the one-time `ui_token` while simultaneously polling the host in case the human
      // answered in the Plexi desktop window.
      if (params?.prompt_in_pi_ui && ctx?.hasUI && typeof ctx?.ui?.select === "function" && choices.length > 0 && uiToken) {
        const picked = await ctx.ui.select(
          `${params.summary}${previewRefs.length ? ` (${previewRefs.join(", ")})` : ""}`,
          choices,
        );
        if (typeof picked === "string" && picked.trim()) {
          const ansRes = await client.answerNeedsYou({
            id: itemId,
            answer: picked.trim(),
            uiToken,
          });
          await lifecycle.report("working", { sessionId: sid });
          if (onUiRefresh) await onUiRefresh(ctx);
          return textResult(ansRes.data ?? { ok: ansRes.ok, answer: picked.trim() }, !ansRes.ok);
        }
      }

      const timeoutMs =
        typeof params?.timeout_ms === "number" ? params.timeout_ms : 30_000;
      const deadline = Date.now() + timeoutMs;
      while (Date.now() < deadline) {
        if (signal?.aborted) break;
        const showRes = await client.showNeedsYou(itemId);
        const item = showRes.data?.item;
        const continuation = showRes.data?.continuation;
        if (item?.resolution) {
          await lifecycle.report("working", { sessionId: sid });
          if (onUiRefresh) await onUiRefresh(ctx);
          return textResult({
            ok: item.resolution === "approved",
            status: item.resolution === "approved" ? "answered" : "denied",
            id: itemId,
            resolution: item.resolution,
            answer: item.answer ?? continuation?.answer ?? null,
            item,
            continuation,
          });
        }
        await sleep(150);
      }

      return textResult({
        ok: true,
        status: "waiting_user",
        id: itemId,
        item: askRes.data.item,
        continuation: askRes.data.continuation,
      });
    },
  };

  const needsYouReconcileTool = {
    name: "plexi_needs_you_reconcile",
    label: "Plexi Needs You Reconcile",
    description:
      "Reconcile open and resolved Needs you items and durable ContinuationRecords for this session, pane, or context (`plexi needs-you reconcile`). Use on session start or before resuming a multi-step operation so completed mutations are never duplicated.",
    parameters: {
      type: "object",
      properties: {
        id: {
          type: "string",
          description: "Optional specific item_id, run_id, operation_id, or correlation_id.",
        },
        session_id: {
          type: "string",
          description: "Optional session_id filter.",
        },
        pane_id: {
          type: "number",
          description: "Optional pane_id filter.",
        },
        context_id: {
          type: "number",
          description: "Optional context_id filter.",
        },
      },
      additionalProperties: false,
    },
    async execute(_toolCallId: string, params: Record<string, any>, _signal: AbortSignal | undefined, _onUpdate: any, ctx: any): Promise<ToolResponse> {
      const cfg = client.getAttachConfig();
      const res = await client.reconcileNeedsYou({
        id: typeof params?.id === "string" ? params.id : undefined,
        sessionId:
          typeof params?.session_id === "string" ? params.session_id : undefined,
        paneId:
          typeof params?.pane_id === "number"
            ? params.pane_id
            : params?.id
              ? undefined
              : cfg.paneId,
        contextId:
          typeof params?.context_id === "number" ? params.context_id : cfg.contextId,
      });
      if (onUiRefresh) {
        await onUiRefresh(ctx);
      }
      return textResult(res.data ?? { ok: false, error: res.error }, !res.ok);
    },
  };

  const hostScreenshotTool = {
    name: "plexi_host_screenshot",
    label: "Plexi Host Screenshot",
    description:
      "Capture a PNG screenshot of the Plexi host window or a specific pane (`plexi host screenshot --output <path>`).",
    parameters: {
      type: "object",
      properties: {
        output_path: {
          type: "string",
          description: "Optional output PNG path (defaults to a temporary path).",
        },
        pane_id: {
          type: "number",
          description: "Optional pane ID to crop the screenshot to.",
        },
      },
      additionalProperties: false,
    },
    async execute(_toolCallId: string, params: Record<string, any>, _signal: AbortSignal | undefined, _onUpdate: any, _ctx: any): Promise<ToolResponse> {
      const outPath =
        typeof params?.output_path === "string" && params.output_path.trim()
          ? params.output_path.trim()
          : join(tmpdir(), `plexi_pi_shot_${Date.now()}.png`);
      const res = await client.takeScreenshot({
        outputPath: outPath,
        paneId: typeof params?.pane_id === "number" ? params.pane_id : undefined,
      });
      return textResult(
        {
          ok: res.ok,
          output_path: outPath,
          error: res.error,
        },
        !res.ok,
      );
    },
  };

  const runTool = {
    name: "plexi_run",
    label: "Plexi Run",
    description:
      "List or run named workspace and global commands (`plexi run --json` / `plexi run <name> [-- <args>...]`). Omit `command` to list merged workspace + global commands (`[{name, scope, description, run, secrets}]`).",
    parameters: {
      type: "object",
      properties: {
        command: {
          type: "string",
          description:
            "Optional command name to run (`plexi run <command>`). Omit to list available workspace and global commands via `plexi run --json`.",
        },
        args: {
          type: "array",
          items: { type: "string" },
          description:
            "Optional positional arguments forwarded after `--` (`$1`, `$2`, ...).",
        },
        timeout_ms: {
          type: "number",
          description: "Optional timeout in milliseconds when running a command (default 30000).",
        },
      },
      additionalProperties: false,
    },
    async execute(_toolCallId: string, params: Record<string, any>, _signal: AbortSignal | undefined, _onUpdate: any, _ctx: any): Promise<ToolResponse> {
      const command =
        typeof params?.command === "string" && params.command.trim()
          ? params.command.trim()
          : undefined;
      if (!command) {
        const res = await client.listCommands();
        return textResult(
          {
            ok: res.ok,
            commands: res.data ?? [],
            error: res.error,
          },
          !res.ok,
        );
      }
      const extraArgs = Array.isArray(params?.args)
        ? params.args.map(String)
        : undefined;
      const timeoutMs =
        typeof params?.timeout_ms === "number" ? params.timeout_ms : undefined;
      const res = await client.runCommand({
        command,
        args: extraArgs,
        timeoutMs,
      });
      return textResult(
        {
          ok: res.ok,
          command,
          exit_code: res.exitCode,
          stdout: res.stdout,
          stderr: res.stderr,
          error: res.error,
        },
        !res.ok,
      );
    },
  };

  return [
    workspaceOverviewTool,
    paneInspectTool,
    appToolsTool,
    appCallTool,
    eventsPollTool,
    needsYouAskTool,
    needsYouReconcileTool,
    hostScreenshotTool,
    runTool,
  ];
}

/** Role names come from the resolved agent definition; the host rechecks authority. */
export function buildAgentTools(client: PlexiCliClient, canCall: string[], canHandoff: string[]) {
  const run = async (args: string[]): Promise<ToolResponse> => {
    const res = await client.runJson(args, 86_405_000);
    return textResult(res.data ?? { ok: false, error: res.error }, !res.ok);
  };
  const calls = [...new Set(canCall)].filter(role => /^[a-z][a-z0-9-]*$/.test(role)).map(role => ({
    name: `agent_call_${role}`,
    label: `Call ${role}`,
    description: `Delegate a task to ${role}. Waits for a validated completion or a blocking question; reply with agent_message.`,
    parameters: {
      type: "object", properties: {
        task: { type: "string" }, worktree: { type: "string" }, async: { type: "boolean" },
      }, required: ["task"], additionalProperties: false,
    },
    async execute(_id: string, params: Record<string, any>): Promise<ToolResponse> {
      const args = ["agent", "call", role, "--task", String(params.task)];
      if (params.worktree) args.push("--worktree", String(params.worktree));
      if (params.async) args.push("--async");
      return run(args);
    },
  }));
  return [...calls, {
    name: "agent_message", label: "Reply to Child Agent",
    description: "Send a structured mailbox reply to a child agent. With wait, return its next completion or question.",
    parameters: {
      type: "object", properties: { pane_id: { type: "integer" }, text: { type: "string" }, wait: { type: "boolean" } },
      required: ["pane_id", "text"], additionalProperties: false,
    },
    async execute(_id: string, params: Record<string, any>): Promise<ToolResponse> {
      const args = ["agent", "message", String(params.pane_id), String(params.text)];
      if (params.wait) args.push("--wait");
      return run(args);
    },
  }, {
    name: "agent_handoff", label: "Hand Off Task",
    description: `Transfer this task to a permitted successor (${canHandoff.join(", ") || "none configured"}).`,
    parameters: {
      type: "object", properties: { role: { type: "string" }, task: { type: "string" } },
      required: ["role", "task"], additionalProperties: false,
    },
    async execute(_id: string, params: Record<string, any>): Promise<ToolResponse> {
      if (!canHandoff.includes(params.role)) return textResult({ ok: false, error: "Role is not in can_handoff" }, true);
      return run(["agent", "handoff", String(params.role), "--task", String(params.task)]);
    },
  }];
}
