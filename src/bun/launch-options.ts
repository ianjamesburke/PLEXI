import type { WorkspaceProfile } from "./workspace-file";

export type LaunchOptions = {
  clean: boolean;
  profile: WorkspaceProfile;
};

function isCleanEnvEnabled(value: string | undefined) {
  if (!value) {
    return false;
  }

  return value === "1" || value.toLowerCase() === "true";
}

export function resolveLaunchOptions(
  argv: string[] = process.argv,
  env: Record<string, string | undefined> = process.env,
): LaunchOptions {
  const clean = argv.includes("--clean") || isCleanEnvEnabled(env.PLEXI_CLEAN);

  return {
    clean,
    profile: clean ? "clean" : "default",
  };
}
