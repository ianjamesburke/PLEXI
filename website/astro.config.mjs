// @ts-check
import { defineConfig } from 'astro/config';
import mdx from '@astrojs/mdx';
import node from '@astrojs/node';
import sitemap from '@astrojs/sitemap';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  site: 'https://plexiapp.com',
  integrations: [mdx(), sitemap({ filter: (page) => !page.endsWith('/kit/') })],
  adapter: node({ mode: 'standalone' }),
  markdown: {
    shikiConfig: { theme: 'tokyo-night' },
  },
  image: {
    service: { entrypoint: 'astro/assets/services/sharp' },
  },
  vite: {
    plugins: [tailwindcss()],
    ssr: { external: ['pg'] },
  },
});
