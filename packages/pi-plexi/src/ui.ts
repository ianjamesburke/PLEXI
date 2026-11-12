import type { WorkspaceOverviewSnapshot } from "./cli-client.ts";
import type { PlexiEventRecord } from "./events.ts";

export const PLEXI_WIDGET_KEY = "plexi-workspace";
export const PLEXI_STATUS_KEY = "plexi";

/**
 * Format compact status bar text for `ctx.ui.setStatus("plexi", ...)`.
 */
export function formatStatusText(snapshot: WorkspaceOverviewSnapshot): string {
  if (!snapshot.attached) {
    return `plexi: detached (${snapshot.status.replace(/^detached_/, "")})`;
  }
  const ctxPart =
    snapshot.contextName
      ? `${snapshot.contextName}#${snapshot.contextId ?? "?"}`
      : `ctx#${snapshot.contextId ?? "?"}`;
  const panePart = `pane:${snapshot.paneId ?? "?"}`;
  const apps = snapshot.panes.filter((p) => p.type === "app").length;
  const needs = snapshot.needsYou.length;
  const needsPart = needs > 0 ? ` · needs-you:${needs}` : "";
  return `plexi[${snapshot.channel}] ${ctxPart} ${panePart} · apps:${apps}${needsPart}`;
}

/**
 * Format multi-line widget lines for `ctx.ui.setWidget("plexi-workspace", lines)`.
 */
export function formatWidgetLines(
  snapshot: WorkspaceOverviewSnapshot,
  recentEvents: PlexiEventRecord[] = [],
): string[] {
  if (!snapshot.attached) {
    return [
      `Plexi Workspace · Detached (${snapshot.status})`,
      snapshot.reason || "No live Plexi host attached to this pane.",
    ];
  }

  const lines: string[] = [];
  const ctxLabel = snapshot.contextName
    ? `${snapshot.contextName} (id=${snapshot.contextId ?? "?"})`
    : `Context ${snapshot.contextId ?? "?"}`;
  lines.push(
    `Plexi Workspace [${snapshot.channel}] · ${ctxLabel} · self pane #${snapshot.paneId ?? "?"}`,
  );

  const paneSummaries = snapshot.panes.map((p) => {
    const marker = Number(p.id) === Number(snapshot.paneId) ? "*" : "";
    const kind = p.type === "app" ? `app:${p.app_id || p.title}` : `term:${p.title || p.id}`;
    return `${marker}#${p.id}(${kind})`;
  });
  if (paneSummaries.length > 0) {
    lines.push(`Panes: ${paneSummaries.join("  ")}`);
  }

  if (snapshot.appTools.length > 0) {
    const toolNames = snapshot.appTools.map((t: any) => {
      const ro = t.read_only ? "ro" : "ask";
      return `${t.app_id}.${t.name}[${ro}]`;
    });
    lines.push(`App tools: ${toolNames.join(", ")}`);
  }

  if (snapshot.needsYou.length > 0) {
    for (const item of snapshot.needsYou.slice(0, 3)) {
      const choices =
        Array.isArray(item.choices) && item.choices.length > 0
          ? ` choices=[${item.choices.join(" | ")}]`
          : "";
      lines.push(`Needs you [${item.kind}] (${item.id}): ${item.summary}${choices}`);
    }
  }

  const pendingCont = snapshot.continuations.filter(
    (c: any) => c.state === "waiting_user" || c.resolution,
  );
  if (pendingCont.length > 0) {
    const summary = pendingCont
      .slice(-2)
      .map(
        (c: any) =>
          `${c.run_id}:${c.state}${c.answer ? `="${c.answer}"` : ""}`,
      )
      .join(", ");
    lines.push(`Continuations: ${summary}`);
  }

  if (recentEvents.length > 0) {
    const last = recentEvents[recentEvents.length - 1];
    lines.push(
      `Last event: [${last.event}] ${last.summary}${last.revision_after ? ` (${last.revision_after})` : ""}`,
    );
  }

  return lines;
}

/**
 * Update Pi's live TUI status bar and workspace widget when UI is present.
 * Safe no-op in headless/JSON modes or when `ctx.ui` is unavailable.
 */
export function applyWorkspaceUi(
  ctx: any,
  snapshot: WorkspaceOverviewSnapshot,
  recentEvents: PlexiEventRecord[] = [],
): void {
  if (!ctx?.ui) return;
  try {
    ctx.ui.setStatus?.(PLEXI_STATUS_KEY, formatStatusText(snapshot));
    ctx.ui.setWidget?.(
      PLEXI_WIDGET_KEY,
      formatWidgetLines(snapshot, recentEvents),
      { placement: "aboveEditor" },
    );
  } catch {
    // Never crash turn execution if UI rendering fails in non-interactive mode.
  }
}
