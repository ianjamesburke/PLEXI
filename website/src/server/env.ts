/**
 * Env access that works under both Astro (SSR, via `import.meta.env`) and
 * vitest/Node (via `process.env`). Reads `process.env` first so tests can set
 * values before importing modules; falls back to `import.meta.env` for Astro's
 * build-time-injected server secrets.
 */

export function readEnv(key: string): string | undefined {
  const fromProcess =
    typeof process !== 'undefined' && process.env ? process.env[key] : undefined;
  if (fromProcess !== undefined && fromProcess !== '') return fromProcess;

  // `import.meta.env` is Vite/Astro-specific and is simply `undefined` under
  // plain Node — optional chaining is enough, no try/catch needed.
  const metaEnv = (import.meta as unknown as { env?: Record<string, string | undefined> }).env;
  const fromMeta = metaEnv?.[key];
  if (fromMeta !== undefined && fromMeta !== '') return fromMeta;

  return undefined;
}

/** Thrown when PUBLIC_SITE_URL is absent — fail fast, no silent fallback. */
export class MissingSiteUrlError extends Error {
  constructor() {
    super(
      'PUBLIC_SITE_URL is not set — it is required to build magic-link verification URLs. ' +
        'Set it in the service environment (e.g. https://plexiapp.com).',
    );
    this.name = 'MissingSiteUrlError';
  }
}

/** Public origin used to build magic-link URLs. Required — no default. */
export function siteUrl(): string {
  const url = readEnv('PUBLIC_SITE_URL');
  if (!url) throw new MissingSiteUrlError();
  return url.replace(/\/+$/, '');
}
