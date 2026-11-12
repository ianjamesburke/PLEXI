import { getCollection } from 'astro:content';

/** Published posts, newest first. Drafts (`draft: true`) get no page, no feed entry and no nav link. */
export async function publishedPosts() {
  const posts = await getCollection('blog', (post) => !post.data.draft);
  return posts.sort((a, b) => b.data.date.valueOf() - a.data.date.valueOf());
}
