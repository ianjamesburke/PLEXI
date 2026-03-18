/// Plexi configuration: reads ~/.plexi/config.json, merges with defaults,
/// and supports per-workspace overrides via a "config" key in workspace files.
///
/// Config is NOT written on first launch. The file only exists if the user
/// creates it manually. When it does exist, valid fields are merged over
/// defaults — invalid fields are silently ignored (the app always starts).

import { TERMINAL_PROFILE } from "./app-constants.js";

const DEFAULT_CONFIG = {
  terminal: {
    fontFamily: TERMINAL_PROFILE.fontFamily,
    fontSize: TERMINAL_PROFILE.fontSize,
    cursorBlink: TERMINAL_PROFILE.cursorBlink,
    cursorStyle: "block",
    lineHeight: TERMINAL_PROFILE.lineHeight,
    letterSpacing: TERMINAL_PROFILE.letterSpacing,
    scrollback: 10000,
    theme: "plexi-dark",
  },
  shell: {
    path: null,
    defaultCwd: null,
  },
  keyboard: {},
};

// Known sections and their field types for validation.
const CONFIG_SCHEMA = {
  terminal: {
    fontFamily: "string",
    fontSize: "number",
    cursorBlink: "boolean",
    cursorStyle: "string",
    lineHeight: "number",
    letterSpacing: "number",
    scrollback: "number",
    theme: "string",
  },
  shell: {
    path: "string|null",
    defaultCwd: "string|null",
  },
  keyboard: null, // free-form string->string map
};

let activeConfig = null;
let configWarnings = [];

export function getDefaultConfig() {
  return JSON.parse(JSON.stringify(DEFAULT_CONFIG));
}

export function getConfigWarnings() {
  return [...configWarnings];
}

/// Load config from the session bridge (Tauri) or return defaults.
export async function loadConfig(sessionBridge) {
  configWarnings = [];

  if (!sessionBridge?.readConfig) {
    activeConfig = getDefaultConfig();
    return activeConfig;
  }

  const stored = await sessionBridge.readConfig();

  if (!stored) {
    // No config file — use defaults. File is only created if the user wants one.
    activeConfig = getDefaultConfig();
    return activeConfig;
  }

  if (typeof stored !== "object" || Array.isArray(stored)) {
    configWarnings.push("config.json is not a JSON object — using defaults");
    activeConfig = getDefaultConfig();
    return activeConfig;
  }

  const { merged, warnings } = validateAndMerge(getDefaultConfig(), stored);
  configWarnings = warnings;
  activeConfig = merged;
  return activeConfig;
}

/// Apply per-workspace config overrides on top of the global config.
export function resolveConfig(workspaceOverrides) {
  const base = activeConfig || getDefaultConfig();
  if (!workspaceOverrides) {
    return base;
  }
  const { merged } = validateAndMerge(base, workspaceOverrides);
  return merged;
}

export function getActiveConfig() {
  return activeConfig || getDefaultConfig();
}

/// Validate and merge user config over defaults. Returns merged result
/// plus an array of human-readable warnings for any invalid fields.
function validateAndMerge(base, overrides) {
  const merged = JSON.parse(JSON.stringify(base));
  const warnings = [];

  for (const section of Object.keys(overrides)) {
    if (!(section in CONFIG_SCHEMA)) {
      warnings.push(`Unknown config section "${section}" — ignored`);
      continue;
    }

    const sectionValue = overrides[section];

    if (typeof sectionValue !== "object" || sectionValue === null || Array.isArray(sectionValue)) {
      warnings.push(`Config section "${section}" should be an object — ignored`);
      continue;
    }

    const schema = CONFIG_SCHEMA[section];

    if (!schema) {
      // Free-form section (e.g. keyboard) — accept as-is
      merged[section] = { ...merged[section], ...sectionValue };
      continue;
    }

    for (const [key, value] of Object.entries(sectionValue)) {
      if (!(key in schema)) {
        warnings.push(`Unknown key "${section}.${key}" — ignored`);
        continue;
      }

      const expectedType = schema[key];

      if (!matchesType(value, expectedType)) {
        const actual = value === null ? "null" : typeof value;
        warnings.push(
          `"${section}.${key}" should be ${expectedType}, got ${actual} — using default`
        );
        continue;
      }

      merged[section][key] = value;
    }
  }

  return { merged, warnings };
}

function matchesType(value, expectedType) {
  if (expectedType.includes("|")) {
    return expectedType.split("|").some((t) => matchesType(value, t));
  }

  if (expectedType === "null") {
    return value === null;
  }

  return typeof value === expectedType;
}
