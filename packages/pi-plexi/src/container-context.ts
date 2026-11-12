import type { AttachConfig } from "./attach.ts";

/**
 * System prompt section name. Rendered as `<plexi_container>…</plexi_container>`
 * by Pi's prompt builder (section names must match `/^[a-z][a-z0-9_-]*$/`).
 */
export const PLEXI_CONTAINER_SECTION = "plexi_container";

/**
 * Describe the Plexi container to the model so it reaches for a Plexi surface
 * before an OS app or an external editor. Returns undefined when this session is
 * not attached to a live Plexi pane, so detached sessions read as plain Pi.
 *
 * Keep this short: it is re-injected on every run and every line taxes the turn.
 * Surface detail belongs in the `plexi-cli` skill; this only has to establish
 * "you are inside Plexi, prefer it" plus the entry points a model reaches for by
 * reflex (`open`, TextEdit, a terminal editor).
 */
export function buildContainerContext(attachConfig: AttachConfig): string | undefined {
  if (!attachConfig.attached) return undefined;
  const pane = attachConfig.paneId !== undefined ? `pane #${attachConfig.paneId}` : "a pane";
  const channel = attachConfig.channel || "unknown";

  return [
    `You are running inside ${pane} of the Plexi host (channel: ${channel}). Plexi is an agent operating system: prefer its native surfaces over OS apps and external tools, and treat the OS as the fallback.`,
    "Open files and media in Plexi, never macOS `open`, TextEdit, or a terminal editor:",
    "- `plexi app open text-editor <path>` — text/markdown/code (.md gets live preview); no path opens a scratch note",
    "- `plexi app open image-viewer <path>` / `video-player <path>` / `audio-player <path>`",
    "- `plexi app open file_browser` — browse the pane's directory",
    "- `plexi note open` — the built-in notes editor; `plexi note \"<text>\"` captures one, `plexi note list` then `plexi note show <path>` reads them, `plexi note rm <path>` retires one",
    "Built-in pane types do not appear in `plexi app list` (installed apps only). Read an open app pane with `plexi pane state <id>`; `pane capture` is terminal-only. Prefer the `plexi_*` tools and the `plexi` skill; the `plexi-cli` skill covers the rest.",
  ].join("\n");
}
