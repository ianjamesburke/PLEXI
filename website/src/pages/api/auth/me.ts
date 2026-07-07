import type { APIRoute } from 'astro';
import { json, requireAccount } from '../../../server/http';

export const prerender = false;

export const GET: APIRoute = async ({ request, cookies }) => {
  try {
    const auth = await requireAccount(request, cookies);
    if (auth instanceof Response) return auth;
    return json({ account_id: auth.account.id, email: auth.account.email }, 200);
  } catch (err) {
    console.error('[api/auth/me] failed to resolve bearer token:', err);
    return json({ error: 'server error' }, 500);
  }
};
