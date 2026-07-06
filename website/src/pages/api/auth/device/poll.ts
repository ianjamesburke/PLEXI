import type { APIRoute } from 'astro';
import { pollDeviceCode, PROVIDER } from '../../../../server/auth';
import { json, parseJsonBody } from '../../../../server/http';

export const prerender = false;

export const POST: APIRoute = async ({ request }) => {
  const body = await parseJsonBody(request);
  if (body instanceof Response) return body;

  const deviceCode = typeof body.device_code === 'string' ? body.device_code : '';
  const pollToken = typeof body.poll_token === 'string' ? body.poll_token : '';
  if (!deviceCode || !pollToken) {
    return json({ error: 'device_code and poll_token are required' }, 400);
  }

  try {
    const result = await pollDeviceCode(deviceCode, pollToken);
    if (result.state === 'pending') {
      return json({ status: 'pending' }, 202);
    }
    if (result.state === 'gone') {
      return json({ error: 'device code expired or already used' }, 410);
    }
    return json(
      {
        schema_version: result.schema_version,
        token: result.token,
        account_id: result.accountId,
        email: result.email,
        provider: PROVIDER,
        issued_at: result.issuedAt,
      },
      200,
    );
  } catch (err) {
    console.error(`[api/auth/device/poll] failed for device_code="${deviceCode}":`, err);
    return json({ error: 'server error' }, 500);
  }
};
