import type { APIRoute } from 'astro';
import { emailMagicLink } from '../../../../server/auth';
import { json, requireAccount } from '../../../../server/http';

export const prerender = false;

export const POST: APIRoute = async ({ request, cookies }) => {
  try {
    const auth = await requireAccount(request, cookies);
    if (auth instanceof Response) return auth;
    await emailMagicLink(auth.account.email, 'account_delete');
    return json(
      { ok: true, message: `We emailed a confirmation link to ${auth.account.email}.` },
      200,
    );
  } catch (err) {
    console.error('[api/auth/delete/start] failed to send delete confirmation:', err);
    return json({ error: 'server error' }, 500);
  }
};
