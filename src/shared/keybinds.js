const PREFIXES = new Set(["all", "global", "performable", "unconsumed"]);
const MODIFIER_ALIASES = {
  cmd: "super",
  command: "super",
  control: "ctrl",
  equal: "=",
  minus: "-",
  opt: "alt",
  option: "alt",
};

const CODE_KEY_ALIASES = {
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
  ArrowUp: "up",
  Equal: "=",
  Minus: "-",
  Slash: "/",
};

function normalizeModifier(token) {
  return MODIFIER_ALIASES[token] || token;
}

function parseAction(actionSpec) {
  const separatorIndex = actionSpec.indexOf(":");
  if (separatorIndex === -1) {
    return {
      name: actionSpec,
      argument: null,
      raw: actionSpec,
    };
  }

  return {
    name: actionSpec.slice(0, separatorIndex),
    argument: actionSpec.slice(separatorIndex + 1),
    raw: actionSpec,
  };
}

function parseTriggerStep(stepSpec) {
  const parts = stepSpec
    .split("+")
    .map((part) => part.trim().toLowerCase())
    .filter(Boolean);
  const key = normalizeModifier(parts.pop());

  if (!key) {
    throw new Error(`Invalid keybind trigger: ${stepSpec}`);
  }

  const modifiers = new Set(parts.map(normalizeModifier));

  return {
    key,
    modifiers: {
      alt: modifiers.has("alt"),
      ctrl: modifiers.has("ctrl"),
      shift: modifiers.has("shift"),
      super: modifiers.has("super"),
    },
    raw: stepSpec,
  };
}

function parseTriggerSpec(triggerSpec) {
  const tokens = triggerSpec
    .split(":")
    .map((token) => token.trim())
    .filter(Boolean);
  const prefixes = {
    all: false,
    global: false,
    performable: false,
    unconsumed: false,
  };
  let sequenceSpec = null;

  for (const token of tokens) {
    const normalized = token.toLowerCase();
    if (!sequenceSpec && PREFIXES.has(normalized)) {
      prefixes[normalized] = true;
      continue;
    }

    sequenceSpec = sequenceSpec ? `${sequenceSpec}:${token}` : token;
  }

  if (!sequenceSpec) {
    throw new Error(`Missing trigger sequence in keybind: ${triggerSpec}`);
  }

  return {
    prefixes,
    sequence: sequenceSpec
      .split(">")
      .map((step) => step.trim())
      .filter(Boolean)
      .map(parseTriggerStep),
    raw: triggerSpec,
  };
}

function parseKeybindSpec(spec) {
  const separatorIndex = spec.indexOf("=");
  if (separatorIndex === -1) {
    throw new Error(`Invalid keybind: ${spec}`);
  }

  const triggerSpec = spec.slice(0, separatorIndex).trim();
  const actionSpec = spec.slice(separatorIndex + 1).trim();

  return {
    ...parseTriggerSpec(triggerSpec),
    action: parseAction(actionSpec),
    spec,
  };
}

function normalizeEventKey(event) {
  const codeKey = CODE_KEY_ALIASES[event.code];
  if (codeKey) {
    return codeKey;
  }

  if (/^Key[A-Z]$/.test(event.code)) {
    return event.code.slice(3).toLowerCase();
  }

  if (/^Digit[0-9]$/.test(event.code)) {
    return event.code.slice(5);
  }

  const key = String(event.key || "").toLowerCase();
  if (key === "arrowleft") return "left";
  if (key === "arrowright") return "right";
  if (key === "arrowup") return "up";
  if (key === "arrowdown") return "down";
  return key;
}

function eventMatchesStep(event, step) {
  const key = normalizeEventKey(event);
  return (
    key === step.key &&
    Boolean(event.altKey) === step.modifiers.alt &&
    Boolean(event.ctrlKey) === step.modifiers.ctrl &&
    Boolean(event.metaKey) === step.modifiers.super &&
    Boolean(event.shiftKey) === step.modifiers.shift
  );
}

export function compileKeybinds(specs) {
  return specs.map(parseKeybindSpec);
}

export function resolveKeybind(event, bindings, options = {}) {
  const eventType = event.type || "keydown";
  if (eventType !== "keydown") {
    return null;
  }

  for (const binding of bindings) {
    if (binding.sequence.length !== 1) {
      continue;
    }

    if (!eventMatchesStep(event, binding.sequence[0])) {
      continue;
    }

    if (binding.prefixes.performable) {
      const canPerform = options.canPerform?.(binding.action, binding) ?? false;
      if (!canPerform) {
        continue;
      }
    }

    return {
      binding,
      action: binding.action,
      consume: !binding.prefixes.unconsumed,
      performable: binding.prefixes.performable,
    };
  }

  return null;
}

export function createPrimaryKeybind({ mac, other }) {
  return process.platform === "darwin" ? mac : other;
}
