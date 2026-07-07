import type { APIRoute } from 'astro';
import { revokeBearer } from '../../../server/auth';
import { json, requireAccount, SESSION_COOKIE } from '../../../server/http';

export const prerender = false;

export const POST: APIRoute = async ({ request, cookies }) => {
  try {
    const auth = await requireAccount(request, cookies);
    if (auth instanceof Response) return auth;
    await revokeBearer(auth.token);
    cookies.delete(SESSION_COOKIE, { path: '/' });
    return json({ ok: true }, 200);
  } catch (err) {
    console.error('[api/auth/revoke] failed to revoke bearer token:', err);
    return json({ error: 'server error' }, 500);
  }
};
