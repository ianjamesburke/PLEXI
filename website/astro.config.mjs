// @ts-check
import { defineConfig } from 'astro/config';
import mdx from '@astrojs/mdx';
import node from '@astrojs/node';
import sitemap from '@astrojs/sitemap';
import tailwindcss from '@tailwindcss/vite';

/** Wraps each markdown table in a scroll container so the table itself stays a real, full-width table. */
function rehypeScrollTables() {
  /** @param {any} node */
  const walk = (node) => {
    if (!node.children) return;
    node.children = node.children.map((/** @type {any} */ child) => {
      walk(child);
      if (child.type !== 'element' || child.tagName !== 'table') return child;
      return { type: 'element', tagName: 'div', properties: { className: ['table-scroll'] }, children: [child] };
    });
  };
  return walk;
}

export default defineConfig({
  site: 'https://plexiapp.com',
  redirects: { '/product': '/tour' },
  integrations: [mdx(), sitemap({ filter: (page) => !page.endsWith('/kit/') })],
  adapter: node({ mode: 'standalone' }),
  markdown: {
    shikiConfig: { theme: 'tokyo-night' },
    rehypePlugins: [rehypeScrollTables],
  },
  image: {
    service: { entrypoint: 'astro/assets/services/sharp' },
  },
  vite: {
    plugins: [tailwindcss()],
    ssr: { external: ['pg'] },
  },
});
