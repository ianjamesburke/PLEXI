import {
  detectAttachment,
  verifyLiveIdentity,
  type AttachConfig,
} from "../src/attach.ts";
import { PlexiCliClient, type ExecRunner } from "../src/cli-client.ts";
import { PlexiEventSubscriber } from "../src/events.ts";
import {
  markPiPlexiPackageActive,
  migrateLegacyPiHook,
  PlexiLifecycleReporter,
  resolveSessionIdFromContext,
} from "../src/lifecycle.ts";
import { buildAgentTools, buildPlexiTools } from "../src/tools.ts";
import { applyWorkspaceUi } from "../src/ui.ts";

export interface PiPlexiExtensionOptions {
  env?: Record<string, string | undefined>;
  runner?: ExecRunner;
  checkSocketExists?: (path: string) => boolean;
  homeDir?: string;
}

export function createPiPlexiExtension(options: PiPlexiExtensionOptions = {}) {
  return function registerPiPlexi(pi: any) {
    markPiPlexiPackageActive();
    migrateLegacyPiHook({ homeDir: options.homeDir });

    let attachConfig: AttachConfig = detectAttachment(options.env ?? process.env, {
      checkSocketExists: options.checkSocketExists,
    });
    const client = new PlexiCliClient(attachConfig, options.runner);
    let lastContext: any = undefined;

    const refreshUi = async (ctx?: any) => {
      const targetCtx = ctx ?? lastContext;
      if (ctx) lastContext = ctx;
      const sid = resolveSessionIdFromContext(targetCtx);
      const snapshot = await client.fetchWorkspaceOverview(sid);
      applyWorkspaceUi(targetCtx, snapshot, events.getRecentEvents(5));
    };

    const events = new PlexiEventSubscriber(attachConfig, {
      onEvent: () => {
        if (lastContext) {
          void refreshUi(lastContext);
        }
      },
    });
    const lifecycle = new PlexiLifecycleReporter(client);

    const tools = buildPlexiTools({
      client,
      events,
      lifecycle,
      onUiRefresh: refreshUi,
    });
    for (const tool of tools) {
      pi.registerTool(tool);
    }

    pi.registerCommand?.("plexi-status", {
      description: "Refresh and display the live Plexi workspace overview and widget.",
      handler: async (_args: string, ctx: any) => {
        lastContext = ctx;
        const sid = resolveSessionIdFromContext(ctx);
        const snapshot = await client.fetchWorkspaceOverview(sid);
        applyWorkspaceUi(ctx, snapshot, events.getRecentEvents(5));
        ctx?.ui?.notify?.(
          snapshot.attached
            ? `Plexi attached (${snapshot.channel}) · pane #${snapshot.paneId} · ${snapshot.appTools.length} app tools`
            : `Plexi detached: ${snapshot.reason || snapshot.status}`,
          snapshot.attached ? "info" : "warning",
        );
      },
    });

    pi.registerCommand?.("plexi-reconcile", {
      description: "Reconcile open and resolved Needs you items and continuations for this session.",
      handler: async (_args: string, ctx: any) => {
        lastContext = ctx;
        const sid = resolveSessionIdFromContext(ctx);
        const res = await client.reconcileNeedsYou({
          sessionId: sid,
          paneId: client.getAttachConfig().paneId,
          contextId: client.getAttachConfig().contextId,
        });
        await refreshUi(ctx);
        const contCount = res.data?.continuations?.length ?? 0;
        const itemCount = res.data?.items?.length ?? 0;
        ctx?.ui?.notify?.(
          `Reconciled ${itemCount} Needs-you item(s) and ${contCount} continuation(s).`,
          "info",
        );
      },
    });

    pi.registerCommand?.("plexi-watch", {
      description: "Subscribe to an app event stream in the current context: /plexi-watch <app_id> <stream>",
      handler: async (args: string, ctx: any) => {
        lastContext = ctx;
        const [appId, stream] = (args || "").trim().split(/\s+/);
        if (!appId || !stream) {
          ctx?.ui?.notify?.("Usage: /plexi-watch <app_id> <stream>", "warning");
          return;
        }
        const sub = events.subscribe(appId, stream);
        ctx?.ui?.notify?.(
          sub.ok ? `Subscribed to ${sub.key}` : `Failed to subscribe: ${sub.reason}`,
          sub.ok ? "info" : "error",
        );
      },
    });

    pi.on("session_start", async (_event: any, ctx: any) => {
      lastContext = ctx;
      attachConfig = detectAttachment(options.env ?? process.env, {
        checkSocketExists: options.checkSocketExists,
      });
      client.setAttachConfig(attachConfig);
      events.setAttachConfig(attachConfig);

      if (attachConfig.attached) {
        const panesRes = await client.listPanes();
        attachConfig = verifyLiveIdentity(
          attachConfig,
          panesRes.ok ? panesRes.data : undefined,
          panesRes.error,
        );
        client.setAttachConfig(attachConfig);
        events.setAttachConfig(attachConfig);
      }

      if (attachConfig.attached && attachConfig.paneId !== undefined) {
        const panes = await client.listPanes();
        const pane = Array.isArray(panes.data) ? panes.data.find((p: any) => Number(p.id ?? p.pane_id) === attachConfig.paneId) : undefined;
        const role = pane?.agent_role;
        if (typeof role === "string" && role) {
          const definition = await client.runJson(["agent", "show", role, "--json"]);
          if (definition.ok) {
            const names = (value: unknown): string[] => Array.isArray(value) ? value.filter((v): v is string => typeof v === "string") : [];
            for (const tool of buildAgentTools(client, names(definition.data?.can_call), names(definition.data?.can_handoff))) {
              pi.registerTool(tool);
            }
          } else {
            ctx?.ui?.notify?.(`Agent tools unavailable: ${definition.error}`, "warning");
          }
        }
      }
      lifecycle.resetSession();
      const sid = resolveSessionIdFromContext(ctx, {
        requirePersisted: true,
        turnStarted: false,
      });
      if (attachConfig.attached) {
        await lifecycle.report("idle", { sessionId: sid, force: true });
      }
      await refreshUi(ctx);
    });

    pi.on("before_agent_start", async (_event: any, ctx: any) => {
      lastContext = ctx;
      lifecycle.markTurnStarted();
      const sid = resolveSessionIdFromContext(ctx);
      await lifecycle.report("working", { sessionId: sid, force: true });
    });

    pi.on("agent_start", async (_event: any, ctx: any) => {
      lastContext = ctx;
      lifecycle.markTurnStarted();
      const sid = resolveSessionIdFromContext(ctx);
      await lifecycle.report("working", { sessionId: sid, force: true });
    });

    pi.on("tool_call", async (event: any, ctx: any) => {
      lastContext = ctx;
      lifecycle.markTurnStarted();
      const sid = resolveSessionIdFromContext(ctx);
      const toolName = event?.toolName || "tool";
      await lifecycle.report("working", {
        detail: String(toolName),
        sessionId: sid,
      });
    });

    pi.on("tool_result", async (_event: any, ctx: any) => {
      lastContext = ctx;
      lifecycle.markTurnStarted();
      const sid = resolveSessionIdFromContext(ctx);
      await lifecycle.report("working", { sessionId: sid });
    });

    pi.on("agent_end", async (_event: any, ctx: any) => {
      lastContext = ctx;
      lifecycle.markTurnStarted();
      const sid = resolveSessionIdFromContext(ctx);
      await lifecycle.report("idle", { sessionId: sid, force: true });
      await refreshUi(ctx);
    });

    pi.on("session_shutdown", async (_event: any, ctx: any) => {
      events.stopAll();
      const sid = resolveSessionIdFromContext(ctx ?? lastContext, {
        requirePersisted: true,
        turnStarted: lifecycle.hasTurnStarted(),
      });
      await lifecycle.report("idle", { sessionId: sid, force: true });
    });

    return {
      client,
      events,
      lifecycle,
      getAttachConfig: () => client.getAttachConfig(),
      refreshUi,
    };
  };
}

const defaultExtension = createPiPlexiExtension();
export default defaultExtension;
