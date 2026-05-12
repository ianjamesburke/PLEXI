// @ts-check
import { defineConfig } from 'astro/config';
import mdx from '@astrojs/mdx';
import node from '@astrojs/node';
import sitemap from '@astrojs/sitemap';
import tailwindcss from '@tailwindcss/vite';

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
  },
});
