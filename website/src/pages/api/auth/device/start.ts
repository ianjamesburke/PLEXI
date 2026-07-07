import type { APIRoute } from 'astro';
import { emailMagicLink, normalizeEmail, pruneExpiredAuthRows, startDeviceFlow } from '../../../../server/auth';
import { isValidEmail, json, parseJsonBody } from '../../../../server/http';

export const prerender = false;

export const POST: APIRoute = async ({ request }) => {
  const body = await parseJsonBody(request);
  if (body instanceof Response) return body;

  const email = typeof body.email === 'string' ? normalizeEmail(body.email) : '';
  if (!isValidEmail(email)) {
    return json({ error: 'invalid email' }, 400);
  }

  try {
    const { deviceCode, pollToken, expiresIn } = await startDeviceFlow(email);
    await emailMagicLink(email, 'device_approve', deviceCode);
    pruneExpiredAuthRows().catch((err) =>
      console.error('[api/auth/device/start] opportunistic prune failed:', err),
    );
    return json(
      {
        device_code: deviceCode,
        poll_token: pollToken,
        expires_in: expiresIn,
        message: `We emailed an approval link to ${email}. Click it to sign in on this device.`,
      },
      200,
    );
  } catch (err) {
    console.error(`[api/auth/device/start] failed for email="${email}":`, err);
    return json({ error: 'server error' }, 500);
  }
};
