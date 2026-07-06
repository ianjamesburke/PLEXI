import type { APIRoute } from 'astro';
import { emailMagicLink, normalizeEmail, pruneExpiredAuthRows } from '../../../server/auth';
import { isValidEmail, json, parseJsonBody } from '../../../server/http';

export const prerender = false;

export const POST: APIRoute = async ({ request }) => {
  const body = await parseJsonBody(request);
  if (body instanceof Response) return body;

  const email = typeof body.email === 'string' ? normalizeEmail(body.email) : '';
  // Web session login is the only purpose callable from this endpoint;
  // device_approve and account_delete are triggered internally.
  const purpose = typeof body.purpose === 'string' ? body.purpose : 'login';

  if (!isValidEmail(email)) {
    return json({ error: 'invalid email' }, 400);
  }
  if (purpose !== 'login') {
    return json({ error: 'unsupported purpose' }, 400);
  }

  try {
    await emailMagicLink(email, 'login');
    pruneExpiredAuthRows().catch((err) =>
      console.error('[api/auth/magic-link] opportunistic prune failed:', err),
    );
    return json({ ok: true, message: `We emailed a sign-in link to ${email}.` }, 200);
  } catch (err) {
    console.error(`[api/auth/magic-link] failed for email="${email}":`, err);
    return json({ error: 'server error' }, 500);
  }
};
