/** JSON response helper — mirrors the shape used across the API routes. */
export function json(body: unknown, status: number): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

import type { AstroCookies } from 'astro';
import { resolveBearer, type Account } from './auth';

/** Name of the httpOnly cookie holding a web-session bearer token. */
export const SESSION_COOKIE = 'plexi_session';

/** Extract a bearer token from the Authorization header, or null. */
export function bearerToken(request: Request): string | null {
  const header = request.headers.get('authorization');
  if (!header) return null;
  const match = header.match(/^Bearer\s+(.+)$/i);
  return match ? match[1].trim() : null;
}

/**
 * Resolve the caller's bearer token from either the Authorization header
 * (CLI/host) or the `plexi_session` cookie (browser web session). Header wins.
 */
export function sessionToken(request: Request, cookies: AstroCookies): string | null {
  return bearerToken(request) ?? cookies.get(SESSION_COOKIE)?.value ?? null;
}

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

export function isValidEmail(email: string): boolean {
  return EMAIL_RE.test(email);
}

/**
 * Parse a JSON request body into an object, or a 400 Response if the body is
 * absent, malformed, or not a JSON object. Caller checks `instanceof Response`.
 */
export async function parseJsonBody(
  request: Request,
): Promise<Record<string, unknown> | Response> {
  let body: unknown;
  try {
    body = await request.json();
  } catch {
    return json({ error: 'invalid json' }, 400);
  }
  if (body === null || typeof body !== 'object') {
    return json({ error: 'invalid json' }, 400);
  }
  return body as Record<string, unknown>;
}

/**
 * Resolve the authenticated caller, or a 401 Response. Returns both the account
 * and the presented token so callers that revoke it (logout) can reuse it.
 * Caller checks `instanceof Response`. DB errors propagate to the caller's catch.
 */
export async function requireAccount(
  request: Request,
  cookies: AstroCookies,
): Promise<{ account: Account; token: string } | Response> {
  const token = sessionToken(request, cookies);
  if (!token) return json({ error: 'missing bearer token' }, 401);
  const account = await resolveBearer(token);
  if (!account) return json({ error: 'invalid or revoked token' }, 401);
  return { account, token };
}
