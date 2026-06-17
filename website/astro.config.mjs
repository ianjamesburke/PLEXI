// @ts-check
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'astro/config';
import mdx from '@astrojs/mdx';
import node from '@astrojs/node';
import sitemap from '@astrojs/sitemap';
import tailwindcss from '@tailwindcss/vite';

// Single source of truth for the displayed version: the workspace
// Cargo.toml [package].version. Read once at build (resolved relative to this
// config file, not cwd) and baked into the bundle via Vite `define`, so there
// is no runtime filesystem read and the value can never drift from the binary.
function readCargoVersion() {
  const cargoTomlPath = fileURLToPath(new URL('../Cargo.toml', import.meta.url));
  const toml = readFileSync(cargoTomlPath, 'utf8');
  let inPackage = false;
  for (const raw of toml.split('\n')) {
    const line = raw.trim();
    if (line.startsWith('[')) {
      inPackage = line === '[package]';
      continue;
    }
    if (inPackage && line.startsWith('version')) {
      const match = line.match(/"([^"]+)"/);
      if (match) return match[1];
    }
  }
  throw new Error(`Could not read [package].version from ${cargoTomlPath}`);
}

const PLEXI_VERSION = readCargoVersion();

// https://astro.build/config
// Note: Astro v6 removed `output: 'hybrid'`. `output: 'static'` (the default)
// now behaves the same way — static pages stay static, routes that export
// `prerender = false` are SSR'd via the configured adapter.
export default defineConfig({
  site: 'https://plexiapp.com',
  integrations: [mdx(), sitemap()],
  adapter: node({ mode: 'standalone' }),
  image: {
    // Astro's default image service is sharp; declared here for clarity.
    service: { entrypoint: 'astro/assets/services/sharp' },
  },
  vite: {
    plugins: [tailwindcss()],
    // better-sqlite3 is a native module; keep it external from the SSR bundle.
    ssr: { external: ['better-sqlite3'] },
    // Bake the Cargo version in as a build-time constant (see version.ts).
    define: { __PLEXI_VERSION__: JSON.stringify(PLEXI_VERSION) },
  },
});
