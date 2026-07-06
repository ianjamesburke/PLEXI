import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { startPg, stopPg, type PgHandle } from './pg';
import {
  approveDeviceCode,
  consumeMagicLink,
  createMagicLink,
  deleteAccount,
  emailMagicLink,
  getOrCreateAccount,
  InvalidMagicLinkError,
  issueBearerToken,
  peekMagicLink,
  pollDeviceCode,
  pruneExpiredAuthRows,
  resolveBearer,
  revokeBearer,
  startDeviceFlow,
} from '../src/server/auth';
import { closePool, getPool, MissingDatabaseUrlError } from '../src/server/db';
import { MissingSiteUrlError, siteUrl } from '../src/server/env';
import { resetEmailTransport, setEmailTransport, type OutboundEmail } from '../src/server/resend';

let pg: PgHandle;

/** Capture the most recent outbound email so tests can read the magic link. */
let lastEmail: OutboundEmail | null = null;

function tokenFromLastEmail(): string {
  if (!lastEmail) throw new Error('no email was sent');
  const match = lastEmail.text.match(/token=([A-Za-z0-9_-]+)/);
  if (!match) throw new Error(`no token in email: ${lastEmail.text}`);
  return match[1];
}

beforeAll(async () => {
  pg = await startPg();
  process.env.DATABASE_URL = pg.url;
  process.env.PUBLIC_SITE_URL = 'https://plexiapp.com';
  setEmailTransport(async (email) => {
    lastEmail = email;
  });
});

afterAll(async () => {
  resetEmailTransport();
  await closePool();
  await stopPg(pg);
});

describe('magic-link auth', () => {
  it('roundtrips and creates an account on first verification', async () => {
    const token = await createMagicLink('Alice@Example.com', 'login');
    const record = await consumeMagicLink(token);
    expect(record.email).toBe('alice@example.com');
    expect(record.purpose).toBe('login');

    const account = await getOrCreateAccount(record.email);
    expect(account.email).toBe('alice@example.com');

    const check = await getPool().query('SELECT email FROM accounts WHERE id = $1', [account.id]);
    expect(check.rows[0].email).toBe('alice@example.com');
  });

  it('rejects a second use of the same link', async () => {
    const token = await createMagicLink('single@example.com', 'login');
    await consumeMagicLink(token);
    await expect(consumeMagicLink(token)).rejects.toBeInstanceOf(InvalidMagicLinkError);
  });

  it('peeks a link without consuming it', async () => {
    const token = await createMagicLink('peek@example.com', 'device_approve');
    expect(await peekMagicLink(token)).toBe('device_approve');
    // Still consumable afterwards — peek did not burn it.
    const record = await consumeMagicLink(token);
    expect(record.email).toBe('peek@example.com');
    // Now consumed, peek returns null.
    expect(await peekMagicLink(token)).toBeNull();
  });

  it('rejects an expired link', async () => {
    const token = await createMagicLink('stale@example.com', 'login');
    // Force expiry into the past.
    const { createHash } = await import('node:crypto');
    const hash = createHash('sha256').update(token).digest('hex');
    await getPool().query(
      `UPDATE magic_link_tokens SET expires_at = now() - interval '1 minute' WHERE token_hash = $1`,
      [hash],
    );
    await expect(consumeMagicLink(token)).rejects.toBeInstanceOf(InvalidMagicLinkError);
  });
});

describe('device-code flow', () => {
  it('runs end to end and yields the bearer token exactly once', async () => {
    const { deviceCode, pollToken } = await startDeviceFlow('device@example.com');

    // Not approved yet.
    expect(await pollDeviceCode(deviceCode, pollToken)).toEqual({ state: 'pending' });

    // The server emails an approval link; the user clicks it.
    await emailMagicLink('device@example.com', 'device_approve', deviceCode);
    const linkToken = tokenFromLastEmail();
    const record = await consumeMagicLink(linkToken);
    expect(record.purpose).toBe('device_approve');
    expect(record.deviceCode).toBe(deviceCode);

    const account = await getOrCreateAccount(record.email);
    await approveDeviceCode(record.deviceCode!, account.id);

    const first = await pollDeviceCode(deviceCode, pollToken);
    expect(first.state).toBe('approved');
    if (first.state !== 'approved') throw new Error('unreachable');
    expect(first.schema_version).toBe(1);
    expect(first.email).toBe('device@example.com');
    expect(first.accountId).toBe(account.id);
    expect(first.token).toBeTruthy();

    // Second poll is consumed → gone.
    expect(await pollDeviceCode(deviceCode, pollToken)).toEqual({ state: 'gone' });

    // The issued token authenticates.
    expect(await resolveBearer(first.token)).not.toBeNull();
  });

  it('returns gone for an unknown poll token', async () => {
    const { deviceCode } = await startDeviceFlow('mismatch@example.com');
    expect(await pollDeviceCode(deviceCode, 'wrong-token')).toEqual({ state: 'gone' });
  });
});

describe('bearer tokens', () => {
  it('resolves then fails after revocation', async () => {
    const account = await getOrCreateAccount('revoke@example.com');
    const { token } = await issueBearerToken(account.id);
    expect(await resolveBearer(token)).toMatchObject({ email: 'revoke@example.com' });

    await revokeBearer(token);
    expect(await resolveBearer(token)).toBeNull();
  });
});

describe('account deletion', () => {
  it('erases the account and tokens, and anonymizes purchases', async () => {
    const account = await getOrCreateAccount('doomed@example.com');
    const { token } = await issueBearerToken(account.id);
    await getPool().query(
      `INSERT INTO purchases (account_id, app_id, amount_cents, currency)
       VALUES ($1, 'demo.app', 1000, 'usd')`,
      [account.id],
    );

    await deleteAccount(account.id);

    // Account gone.
    const acct = await getPool().query('SELECT 1 FROM accounts WHERE id = $1', [account.id]);
    expect(acct.rowCount).toBe(0);
    // Token no longer resolves.
    expect(await resolveBearer(token)).toBeNull();
    // Purchase survives, anonymized.
    const purchase = await getPool().query(
      `SELECT account_id FROM purchases WHERE app_id = 'demo.app'`,
    );
    expect(purchase.rowCount).toBe(1);
    expect(purchase.rows[0].account_id).toBeNull();
  });
});

describe('pruning', () => {
  it('deletes expired device codes and magic links past the grace interval', async () => {
    // Seed a device code and magic link already well past expiry.
    await getPool().query(
      `INSERT INTO device_codes (code, poll_token_hash, email, status, expires_at)
       VALUES ('stale-code', 'stale-hash', 'prune@example.com', 'expired', now() - interval '2 days')`,
    );
    await getPool().query(
      `INSERT INTO magic_link_tokens (token_hash, email, purpose, expires_at)
       VALUES ('stale-link', 'prune@example.com', 'login', now() - interval '2 days')`,
    );

    await pruneExpiredAuthRows();

    const codes = await getPool().query(`SELECT 1 FROM device_codes WHERE code = 'stale-code'`);
    expect(codes.rowCount).toBe(0);
    const links = await getPool().query(
      `SELECT 1 FROM magic_link_tokens WHERE token_hash = 'stale-link'`,
    );
    expect(links.rowCount).toBe(0);
  });
});

describe('siteUrl', () => {
  it('throws when PUBLIC_SITE_URL is unset', () => {
    const saved = process.env.PUBLIC_SITE_URL;
    delete process.env.PUBLIC_SITE_URL;
    try {
      expect(() => siteUrl()).toThrow(MissingSiteUrlError);
    } finally {
      process.env.PUBLIC_SITE_URL = saved;
    }
  });
});

describe('configuration', () => {
  it('throws when DATABASE_URL is missing', async () => {
    // Run last: this tears down the shared pool and clears the env var.
    await closePool();
    const saved = process.env.DATABASE_URL;
    delete process.env.DATABASE_URL;
    try {
      expect(() => getPool()).toThrow(MissingDatabaseUrlError);
    } finally {
      process.env.DATABASE_URL = saved;
    }
  });
});
