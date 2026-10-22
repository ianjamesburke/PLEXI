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

  const switchMode = async (mode: (typeof MODEL_MODES)[number], ctx: any) => {
    const modelName = `plexi/${mode.id}`;
    const model = ctx?.modelRegistry?.find("plexi", mode.id);
    if (!model) {
      ctx?.ui?.notify?.(`Unable to switch to ${modelName}: model not found.`, "error");
      return;
    }

    try {
      if (!await pi.setModel(model)) {
        ctx?.ui?.notify?.(`Unable to switch to ${modelName}.`, "error");
        return;
      }
    } catch (error) {
      const reason = error instanceof Error ? `: ${error.message}` : "";
      ctx?.ui?.notify?.(`Unable to switch to ${modelName}${reason}`, "error");
      return;
    }

    pi.setThinkingLevel(mode.thinking);
    ctx?.ui?.notify?.(`Model: ${mode.name} (${modelName})`);
  };

  const modelOptions = (ctx: any) => MODEL_MODES.map((mode) => {
    const current = ctx?.model?.provider === "plexi" && ctx?.model?.id === mode.id;
    return `${mode.name} — ${mode.provider}/${mode.model} (thinking ${mode.thinking})${current ? " (current)" : ""}`;
  });

  const selectMode = async (args: string, ctx: any) => {
    const requested = args.trim().toLowerCase();
    if (requested) {
      const mode = MODEL_MODES.find((candidate) => candidate.id === requested);
      if (!mode) {
        ctx?.ui?.notify?.("Unknown model. Choose: fast, slow.", "error");
        return;
      }
      await switchMode(mode, ctx);
      return;
    }

    const options = modelOptions(ctx);
    const selected = await ctx?.ui?.select?.("Model", options);
    const index = selected === undefined ? -1 : options.indexOf(selected);
    if (index >= 0) await switchMode(MODEL_MODES[index], ctx);
  };

  for (const mode of MODEL_MODES) {
    pi.registerCommand?.(mode.id, {
      description: `Switch to ${mode.name} (plexi/${mode.id}).`,
      handler: async (_args: string, ctx: any) => switchMode(mode, ctx),
    });
  }

  pi.registerCommand?.("models", {
    description: "Choose FAST or SLOW.",
    getArgumentCompletions: (prefix: string) => MODEL_MODES
      .filter((mode) => mode.id.startsWith(prefix.toLowerCase()))
      .map((mode) => ({ value: mode.id, label: mode.id })),
    handler: selectMode,
  });
}
