/** Explicit bootstrap modes. No prompt classification or fallback. */
export const MODEL_MODES = [
  { id: "fast", name: "FAST", provider: "openrouter", model: "xiaomi/mimo-v2.6-pro", thinking: "medium" },
  { id: "slow", name: "SLOW", provider: "openrouter", model: "openai/gpt-6-astra", thinking: "high" },
] as const;

export function registerModelModes(pi: any): void {
  for (const mode of MODEL_MODES) {
    pi.registerVirtualModel({
      provider: "plexi", id: mode.id, name: mode.name,
      thinkingLevels: [mode.thinking],
      route(_request: unknown, ctx: any) {
        const target = `${mode.provider}/${mode.model}`;
        const model = ctx.modelRegistry.find(mode.provider, mode.model);
        if (!model) throw new Error(`Plexi ${mode.name}: missing target ${target} (thinking ${mode.thinking})`);
        // Pi 1.3.0 getSupportedThinkingLevels: ordinary levels are supported
        // on reasoning models unless explicitly mapped to null.
        if (!model.reasoning || model.thinkingLevelMap?.[mode.thinking] === null) {
          throw new Error(`Plexi ${mode.name}: ${target} does not support thinking ${mode.thinking}`);
        }
        console.info(`[pi-plexi] ${mode.name} -> ${target} thinking=${mode.thinking}`);
        return { model, thinkingLevel: mode.thinking };
      },
    });
  }
}
