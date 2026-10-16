export {
  detectAttachment,
  inferChannelFromSocketPath,
  resolvePlexiBinary,
  verifyLiveIdentity,
  type AttachConfig,
  type AttachStatus,
} from "./src/attach.ts";
export {
  PlexiCliClient,
  type CliResult,
  type ExecRunner,
  type WorkspaceOverviewSnapshot,
} from "./src/cli-client.ts";
export { PlexiEventSubscriber, type PlexiEventRecord } from "./src/events.ts";
export {
  isLegacyPlexiPiExtensionContent,
  LEGACY_PI_EXTENSION_FILENAME,
  markPiPlexiPackageActive,
  migrateLegacyPiHook,
  PLEXI_PI_PACKAGE_ACTIVE_GLOBAL,
  PlexiLifecycleReporter,
  resolveSessionIdFromContext,
} from "./src/lifecycle.ts";
export { buildPlexiTools, type PlexiToolDeps, type ToolResponse } from "./src/tools.ts";
export {
  applyWorkspaceUi,
  formatStatusText,
  formatWidgetLines,
  PLEXI_STATUS_KEY,
  PLEXI_WIDGET_KEY,
} from "./src/ui.ts";
export {
  createPiPlexiExtension,
  default as piPlexiExtension,
  type PiPlexiExtensionOptions,
} from "./extensions/index.ts";
