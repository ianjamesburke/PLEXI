import crypto from 'node:crypto';
import { getClient, query } from './db';
import { siteUrl } from './env';
import { sendMagicLink, type MagicLinkPurpose } from './resend';

const MAGIC_LINK_TTL_MS = 15 * 60 * 1000; // 15 minutes
const DEVICE_CODE_TTL_MS = 10 * 60 * 1000; // 10 minutes

/** The provider string the host stores in `AccountSession.provider`. */
export const PROVIDER = 'plexi';

export class InvalidMagicLinkError extends Error {
  constructor() {
    super('magic link is invalid, expired, or already used');
    this.name = 'InvalidMagicLinkError';
  }
}

export class DeviceCodeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'DeviceCodeError';
  }
}

export interface Account {
  id: string;
  email: string;
}

function generateToken(): string {
  return crypto.randomBytes(32).toString('base64url');
}

function hashToken(token: string): string {
  return crypto.createHash('sha256').update(token).digest('hex');
}

/** Canonical email form used everywhere accounts are keyed by address. */
export function normalizeEmail(email: string): string {
  return email.trim().toLowerCase();
}

/** Build the emailed verification URL for a magic-link token. */
export function verifyUrl(token: string): string {
  return `${siteUrl()}/auth/verify?token=${encodeURIComponent(token)}`;
}

/** Get-or-create an account by lowercase-normalized email. */
export async function getOrCreateAccount(email: string): Promise<Account> {
  const norm = normalizeEmail(email);
  // DO NOTHING (not DO UPDATE) so a returning-user login leaves no dead tuple.
  const inserted = await query<Account>(
    `INSERT INTO accounts (email) VALUES ($1)
     ON CONFLICT (email) DO NOTHING
     RETURNING id, email`,
    [norm],
  );
  if (inserted.rowCount && inserted.rows[0]) {
    console.info(`[auth] account created account_id=${inserted.rows[0].id} email=${norm}`);
    return inserted.rows[0];
  }
  const existing = await query<Account>(
    `SELECT id, email FROM accounts WHERE email = $1`,
    [norm],
  );
  if (existing.rowCount === 0) {
    throw new Error(`getOrCreateAccount: account vanished for email="${norm}"`);
  }
  return existing.rows[0];
}

/**
 * Create a purpose-tagged, single-use magic link and return the raw token.
 * Only the SHA-256 hash is persisted.
 */
export async function createMagicLink(
  email: string,
  purpose: MagicLinkPurpose,
  deviceCode?: string,
): Promise<string> {
  const token = generateToken();
  const expires = new Date(Date.now() + MAGIC_LINK_TTL_MS);
  await query(
    `INSERT INTO magic_link_tokens (token_hash, email, purpose, device_code, expires_at)
     VALUES ($1, $2, $3, $4, $5)`,
    [hashToken(token), normalizeEmail(email), purpose, deviceCode ?? null, expires],
  );
  return token;
}

export interface MagicLinkRecord {
  email: string;
  purpose: MagicLinkPurpose;
  deviceCode: string | null;
}

/**
 * Atomically consume a magic link. Marks it used and returns its record.
 * Throws {@link InvalidMagicLinkError} if unknown, expired, or already used.
 */
export async function consumeMagicLink(token: string): Promise<MagicLinkRecord> {
  const res = await query<{
    email: string;
    purpose: MagicLinkPurpose;
    device_code: string | null;
  }>(
    `UPDATE magic_link_tokens SET used_at = now()
     WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
     RETURNING email, purpose, device_code`,
    [hashToken(token)],
  );
  if (res.rowCount === 0) throw new InvalidMagicLinkError();
  const row = res.rows[0];
  return { email: row.email, purpose: row.purpose, deviceCode: row.device_code };
}

/**
 * Inspect a magic link without consuming it — used to render a confirmation
 * page on GET so email prefetchers/scanners cannot burn the link or trigger a
 * side effect. Returns the purpose if the link is valid and unused, else null.
 */
export async function peekMagicLink(token: string): Promise<MagicLinkPurpose | null> {
  const res = await query<{ purpose: MagicLinkPurpose }>(
    `SELECT purpose FROM magic_link_tokens
     WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()`,
    [hashToken(token)],
  );
  return res.rowCount ? res.rows[0].purpose : null;
}

export interface BearerToken {
  token: string;
  issuedAt: string;
}

/** Issue a fresh opaque bearer token for an account. Stored hashed. */
export async function issueBearerToken(accountId: string): Promise<BearerToken> {
  const token = generateToken();
  const res = await query<{ issued_at: Date }>(
    `INSERT INTO auth_tokens (token_hash, account_id) VALUES ($1, $2)
     RETURNING issued_at`,
    [hashToken(token), accountId],
  );
  console.info(`[auth] bearer token issued account_id=${accountId}`);
  return { token, issuedAt: res.rows[0].issued_at.toISOString() };
}

/** Resolve a presented bearer token to its account, or null if revoked/unknown. */
export async function resolveBearer(token: string): Promise<Account | null> {
  const res = await query<Account>(
    `SELECT a.id, a.email FROM auth_tokens t
     JOIN accounts a ON a.id = t.account_id
     WHERE t.token_hash = $1 AND t.revoked_at IS NULL`,
    [hashToken(token)],
  );
  return res.rowCount ? res.rows[0] : null;
}

/** Revoke a presented bearer token. Idempotent. */
export async function revokeBearer(token: string): Promise<void> {
  const res = await query(
    `UPDATE auth_tokens SET revoked_at = now()
     WHERE token_hash = $1 AND revoked_at IS NULL
     RETURNING account_id`,
    [hashToken(token)],
  );
  if (res.rowCount) {
    console.info(`[auth] bearer token revoked account_id=${res.rows[0].account_id}`);
  }
}

export interface DeviceFlowStart {
  deviceCode: string;
  pollToken: string;
  expiresIn: number;
}

/** Begin a CLI device-code flow. Returns the device code and poll token. */
export async function startDeviceFlow(email: string): Promise<DeviceFlowStart> {
  const deviceCode = generateToken();
  const pollToken = generateToken();
  const expires = new Date(Date.now() + DEVICE_CODE_TTL_MS);
  await query(
    `INSERT INTO device_codes (code, poll_token_hash, email, status, expires_at)
     VALUES ($1, $2, $3, 'pending', $4)`,
    [deviceCode, hashToken(pollToken), normalizeEmail(email), expires],
  );
  return { deviceCode, pollToken, expiresIn: Math.floor(DEVICE_CODE_TTL_MS / 1000) };
}

/**
 * Approve a pending device code (called after its magic link is consumed).
 * Binds the account and flips status pending → approved.
 */
export async function approveDeviceCode(deviceCode: string, accountId: string): Promise<void> {
  const res = await query(
    `UPDATE device_codes SET status = 'approved', account_id = $2
     WHERE code = $1 AND status = 'pending' AND expires_at > now()
     RETURNING code`,
    [deviceCode, accountId],
  );
  if (res.rowCount === 0) {
    throw new DeviceCodeError('device code is not pending or has expired');
  }
  console.info(`[auth] device code approved account_id=${accountId}`);
}

export type PollResult =
  | { state: 'pending' }
  | { state: 'gone' }
  | {
      state: 'approved';
      /** Wire-contract version of the approved payload (AccountSession). */
      schema_version: 1;
      token: string;
      accountId: string;
      email: string;
      issuedAt: string;
    };

/**
 * Poll a device code. Returns `pending` until approved, then `approved` with a
 * bearer token exactly once (flipping status approved → consumed), and `gone`
 * for expired/consumed codes or an unknown/mismatched poll token.
 */
export async function pollDeviceCode(
  deviceCode: string,
  pollToken: string,
): Promise<PollResult> {
  const found = await query<{
    status: string;
    account_id: string | null;
    expires_at: Date;
  }>(
    `SELECT status, account_id, expires_at FROM device_codes
     WHERE code = $1 AND poll_token_hash = $2`,
    [deviceCode, hashToken(pollToken)],
  );
  if (found.rowCount === 0) return { state: 'gone' };

  const row = found.rows[0];
  const expired = row.expires_at.getTime() < Date.now();

  if (row.status === 'pending') {
    return expired ? { state: 'gone' } : { state: 'pending' };
  }

  if (row.status === 'approved' && !expired) {
    // Claim the approval and issue the token on one client so a failed insert
    // rolls the claim back — the code stays approved and a retry can succeed.
    const client = await getClient();
    try {
      await client.query('BEGIN');
      const claim = await client.query<{ account_id: string | null }>(
        `UPDATE device_codes SET status = 'consumed'
         WHERE code = $1 AND status = 'approved' AND expires_at > now()
         RETURNING account_id`,
        [deviceCode],
      );
      if (claim.rowCount === 0 || !claim.rows[0].account_id) {
        await client.query('ROLLBACK');
        return { state: 'gone' };
      }
      const accountId = claim.rows[0].account_id;
      const acct = await client.query<{ email: string }>(
        `SELECT email FROM accounts WHERE id = $1`,
        [accountId],
      );
      if (acct.rowCount === 0) {
        await client.query('ROLLBACK');
        return { state: 'gone' };
      }
      const token = generateToken();
      const ins = await client.query<{ issued_at: Date }>(
        `INSERT INTO auth_tokens (token_hash, account_id) VALUES ($1, $2)
         RETURNING issued_at`,
        [hashToken(token), accountId],
      );
      await client.query('COMMIT');
      const email = acct.rows[0].email;
      console.info(
        `[auth] device code approved, bearer token issued account_id=${accountId} email=${email}`,
      );
      return {
        state: 'approved',
        schema_version: 1,
        token,
        accountId,
        email,
        issuedAt: ins.rows[0].issued_at.toISOString(),
      };
    } catch (err) {
      await client.query('ROLLBACK');
      console.error(`[auth] pollDeviceCode claim failed for device_code:`, err);
      throw err;
    } finally {
      client.release();
    }
  }

  // approved-but-expired | consumed | expired
  return { state: 'gone' };
}

/**
 * Erase an account: delete its tokens (cascade) and device codes, revoke
 * pending magic links, and anonymize purchases (null account_id) so the sales
 * record survives. Runs in a single transaction.
 */
export async function deleteAccount(accountId: string): Promise<void> {
  const client = await getClient();
  try {
    await client.query('BEGIN');
    const acct = await client.query<{ email: string }>(
      `SELECT email FROM accounts WHERE id = $1`,
      [accountId],
    );
    const email = acct.rows[0]?.email;
    await client.query(`UPDATE purchases SET account_id = NULL WHERE account_id = $1`, [accountId]);
    await client.query(
      `DELETE FROM device_codes WHERE account_id = $1${email ? ' OR email = $2' : ''}`,
      email ? [accountId, email] : [accountId],
    );
    if (email) {
      await client.query(`DELETE FROM magic_link_tokens WHERE email = $1`, [email]);
    }
    await client.query(`DELETE FROM accounts WHERE id = $1`, [accountId]); // cascades auth_tokens
    await client.query('COMMIT');
    console.info(`[auth] account deleted account_id=${accountId}`);
  } catch (err) {
    await client.query('ROLLBACK');
    console.error(`[auth] deleteAccount failed for account_id="${accountId}":`, err);
    throw err;
  } finally {
    client.release();
  }
}

/**
 * Opportunistically delete rows past their useful life: expired device codes
 * and magic links (past a grace interval), and long-revoked auth tokens. Called
 * fire-and-forget from auth endpoints; failures are logged, never surfaced.
 */
export async function pruneExpiredAuthRows(): Promise<void> {
  const client = await getClient();
  try {
    await client.query('BEGIN');
    await client.query(
      `DELETE FROM device_codes WHERE expires_at < now() - interval '1 day'`,
    );
    await client.query(
      `DELETE FROM magic_link_tokens WHERE expires_at < now() - interval '1 day'`,
    );
    await client.query(
      `DELETE FROM auth_tokens WHERE revoked_at IS NOT NULL AND revoked_at < now() - interval '30 days'`,
    );
    await client.query('COMMIT');
  } catch (err) {
    await client.query('ROLLBACK');
    console.error('[auth] pruneExpiredAuthRows failed:', err);
    throw err;
  } finally {
    client.release();
  }
}

/**
 * Send a magic link for a purpose, creating the token first. Wraps token
 * creation + email so endpoints call one function.
 */
export async function emailMagicLink(
  email: string,
  purpose: MagicLinkPurpose,
  deviceCode?: string,
): Promise<void> {
  const norm = normalizeEmail(email);
  const token = await createMagicLink(email, purpose, deviceCode);
  await sendMagicLink(norm, verifyUrl(token), purpose);
  console.info(`[auth] magic link sent purpose=${purpose} email=${norm}`);
}
