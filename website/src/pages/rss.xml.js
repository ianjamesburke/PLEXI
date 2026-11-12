import rss from '@astrojs/rss';
import { publishedPosts } from '../lib/blog';

export async function GET(context) {
  const posts = await publishedPosts();
  return rss({
    title: 'Plexi',
    description: 'Notes from building Plexi.',
    site: context.site,
    items: posts.map((post) => ({
      title: post.data.title,
      description: post.data.description,
      pubDate: post.data.date,
      link: `/blog/${post.id}/`,
    })),
  });
}
