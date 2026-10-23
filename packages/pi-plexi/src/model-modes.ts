/** Built-in safe targets used until Plexi config has been read. */
export const DEFAULT_MODEL_MODES = [
  { id: "fast", name: "FAST", provider: "openrouter", model: "deepseek/deepseek-v4.1-flash", thinking: "low" },
  { id: "slow", name: "SLOW", provider: "openrouter", model: "openai/gpt-6-astra", thinking: "high" },
] as const;
type Mode = { id: "fast" | "slow"; name: "FAST" | "SLOW"; provider: string; model: string; thinking: string };
const THINKING = new Set(["off", "minimal", "low", "medium", "high", "xhigh"]);
export function modesFromConfig(value: any): Mode[] | undefined {
  if (!value || typeof value !== "object") return undefined;
  const modes = DEFAULT_MODEL_MODES.map((fallback) => {
    const input = value[fallback.id];
    if (!input || typeof input.provider !== "string" || typeof input.model !== "string" || typeof input.thinking !== "string" || !input.provider || !input.model || /\s/.test(input.provider) || /\s/.test(input.model) || !THINKING.has(input.thinking)) return undefined;
    return { ...fallback, provider: input.provider, model: input.model, thinking: input.thinking };
  });
  return modes.every(Boolean) ? modes as Mode[] : undefined;
}
export function registerModelModes(pi: any) {
  let modes: Mode[] = [...DEFAULT_MODEL_MODES];
  const current = () => modes;
  for (const alias of DEFAULT_MODEL_MODES) pi.registerVirtualModel({
    provider: "plexi", id: alias.id, name: alias.name,
    get thinkingLevels() { return [current().find((mode) => mode.id === alias.id)!.thinking]; },
    route(_request: unknown, ctx: any) {
      const mode = current().find((candidate) => candidate.id === alias.id)!;
      const target = `${mode.provider}/${mode.model}`, model = ctx.modelRegistry.find(mode.provider, mode.model);
      if (!model) throw new Error(`Plexi ${mode.name}: missing target ${target} (thinking ${mode.thinking})`);
      if (mode.thinking !== "off" && (!model.reasoning || model.thinkingLevelMap?.[mode.thinking] === null)) throw new Error(`Plexi ${mode.name}: ${target} does not support thinking ${mode.thinking}`);
      console.info(`[pi-plexi] ${mode.name} -> ${target} thinking=${mode.thinking}`);
      return { model, thinkingLevel: mode.thinking };
    },
  });
  const switchMode = async (mode: Mode, ctx: any) => {
    const modelName = `plexi/${mode.id}`, model = ctx?.modelRegistry?.find("plexi", mode.id);
    if (!model) return void ctx?.ui?.notify?.(`Unable to switch to ${modelName}: model not found.`, "error");
    try { if (!await pi.setModel(model)) return void ctx?.ui?.notify?.(`Unable to switch to ${modelName}.`, "error"); }
    catch (error) { return void ctx?.ui?.notify?.(`Unable to switch to ${modelName}${error instanceof Error ? `: ${error.message}` : ""}`, "error"); }
    pi.setThinkingLevel(mode.thinking); ctx?.ui?.notify?.(`Model: ${mode.name} (${mode.provider}/${mode.model}, thinking ${mode.thinking})`);
  };
  const selectMode = async (args: string, ctx: any) => {
    const requested = args.trim().toLowerCase();
    if (requested) { const mode = current().find((candidate) => candidate.id === requested); if (!mode) return void ctx?.ui?.notify?.("Unknown model. Choose: fast, slow.", "error"); return switchMode(mode, ctx); }
    const options = current().map((mode) => `${mode.name} — ${mode.provider}/${mode.model} (thinking ${mode.thinking})${ctx?.model?.provider === "plexi" && ctx?.model?.id === mode.id ? " (current)" : ""}`);
    const selected = await ctx?.ui?.select?.("Model", options), index = selected === undefined ? -1 : options.indexOf(selected); if (index >= 0) await switchMode(current()[index], ctx);
  };
  for (const id of ["fast", "slow"] as const) pi.registerCommand?.(id, { description: `Switch to ${id.toUpperCase()} (plexi/${id}).`, handler: async (_: string, ctx: any) => switchMode(current().find((mode) => mode.id === id)!, ctx) });
  pi.registerCommand?.("models", { description: "Choose FAST or SLOW.", getArgumentCompletions: (prefix: string) => current().filter((mode) => mode.id.startsWith(prefix.toLowerCase())).map((mode) => ({ value: mode.id, label: mode.id })), handler: selectMode });
  return { setModes(next: Mode[]) { modes = next; }, modes: current };
}
