import type { APIRoute } from 'astro';
import { addSubscriber } from '../../server/db';

export const prerender = false;

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function json(body: unknown, status: number): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

export const POST: APIRoute = async ({ request }) => {
  let payload: unknown;
  try {
    payload = await request.json();
  } catch (err) {
    console.error('[api/subscribe] failed to parse JSON body:', err);
    return json({ error: 'invalid json' }, 400);
  }

  const data = payload as { email?: unknown; source?: unknown } | null;
  const email = typeof data?.email === 'string' ? data.email.trim().toLowerCase() : '';
  const source = typeof data?.source === 'string' ? data.source : undefined;

  if (!EMAIL_RE.test(email)) {
    return json({ error: 'invalid email' }, 400);
  }

  try {
    addSubscriber(email, source ?? 'web');
  } catch (err) {
    console.error(`[api/subscribe] addSubscriber threw for email="${email}":`, err);
    return json({ error: 'server error' }, 500);
  }

  return json({ ok: true }, 200);
};

export const ALL: APIRoute = async ({ request }) => {
  if (request.method === 'POST') {
    // Should never reach here — POST is exported above — but keep the guard tight.
    return json({ error: 'method not allowed' }, 405);
  }
  return new Response(JSON.stringify({ error: 'method not allowed' }), {
    status: 405,
    headers: { 'content-type': 'application/json', allow: 'POST' },
  });
};
