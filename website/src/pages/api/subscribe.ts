import type { APIRoute } from 'astro';
import { addSubscriber } from '../../server/resend';
import { addSubscriber as addSubscriberRow } from '../../server/db';
import { normalizeEmail } from '../../server/auth';
import { isValidEmail, json, parseJsonBody } from '../../server/http';

export const prerender = false;

export const POST: APIRoute = async ({ request }) => {
  const body = await parseJsonBody(request);
  if (body instanceof Response) return body;

  const email = typeof body.email === 'string' ? normalizeEmail(body.email) : '';
  const source = typeof body.source === 'string' ? body.source : undefined;

  if (!isValidEmail(email)) {
    return json({ error: 'invalid email' }, 400);
  }

  // Resend is the source of truth for subscribers — fail the request if it errors.
  try {
    await addSubscriber(email);
  } catch (err) {
    console.error(`[api/subscribe] Resend addSubscriber failed for email="${email}":`, err);
    return json({ error: 'server error' }, 500);
  }

  // The Postgres row is a best-effort mirror; a failure never fails the request.
  try {
    await addSubscriberRow(email, source);
  } catch (err) {
    console.error(
      `[api/subscribe] DB subscriber mirror failed for email="${email}" source="${source ?? ''}":`,
      err,
    );
  }

  return json({ ok: true }, 200);
};

export const ALL: APIRoute = async () => {
  return new Response(JSON.stringify({ error: 'method not allowed' }), {
    status: 405,
    headers: { 'content-type': 'application/json', allow: 'POST' },
  });
};
