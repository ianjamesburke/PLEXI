import type { APIRoute } from 'astro';
import { getCollection } from 'astro:content';
import registry from '../../public/registry/v1/index.json';

export const prerender = true;

export const GET: APIRoute = async ({ site }) => {
  const [docs, posts] = await Promise.all([
    getCollection('docs'),
    getCollection('blog', ({ data }) => !data.draft),
  ]);
  const paths = [
    '/', '/download', '/docs', '/blog', '/apps', '/support',
    '/legal/privacy', '/legal/terms', '/legal/refunds', '/legal/dmca',
    ...registry.apps.filter((app) => app.visibility === 'public').map((app) => '/apps/' + app.id),
    ...docs.map((doc) => '/docs/' + doc.id),
    ...posts.map((post) => '/blog/' + post.id + '/'),
  ];
  const escape = (value: string) => value.replace(/[<>&"']/g, (char) =>
    ({ '<': '&lt;', '>': '&gt;', '&': '&amp;', '"': '&quot;', "'": '&apos;' })[char]!);
  const urls = paths.map((path) => '<url><loc>' + escape(new URL(path, site).href) + '</loc></url>');
  console.info('[sitemap] Generated published-page sitemap');
  return new Response('<?xml version="1.0" encoding="UTF-8"?>\n' +
    '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">' + urls.join('') + '</urlset>',
    { headers: { 'Content-Type': 'application/xml; charset=utf-8' } });
};
