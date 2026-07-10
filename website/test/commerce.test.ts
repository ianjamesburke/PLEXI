import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { Webhook } from 'standardwebhooks';
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import { startPg, stopPg, type PgHandle } from './pg';
import { getOrCreateAccount, issueBearerToken } from '../src/server/auth';
import { closePool, query } from '../src/server/db';
import {
  getPurchaseState,
  hasActiveSubscription,
  hasEntitlement,
  formatPrice,
} from '../src/server/commerce';
import { resetCheckoutClient, setCheckoutClient } from '../src/server/polar';
import { resetArtifactStore, setArtifactStore } from '../src/server/storage';
import { POST as checkoutPOST } from '../src/pages/api/commerce/checkout';
import { POST as webhookPOST } from '../src/pages/api/commerce/webhook/polar';
import { GET as artifactGET } from '../src/pages/api/registry/artifact/[appId]';
import { GET as purchaseGET } from '../src/pages/api/commerce/purchase/[id]';

const WEBHOOK_SECRET = 'whsec_test_secret_0339';
const APP_ID = 'reviewed-notes';
const FIXTURES = join(dirname(fileURLToPath(import.meta.url)), 'fixtures', 'polar');

let pg: PgHandle;

/** Sign a raw body exactly the way Polar / validateEvent expect (secret utf8→base64). */
function signPolar(body: string): Record<string, string> {
  const wh = new Webhook(Buffer.from(WEBHOOK_SECRET, 'utf-8').toString('base64'));
  const id = `msg_${Math.random().toString(36).slice(2)}`;
  const timestamp = new Date();
  const signature = wh.sign(id, timestamp, body);
  return {
    'webhook-id': id,
    'webhook-timestamp': String(Math.floor(timestamp.getTime() / 1000)),
    'webhook-signature': signature,
    'content-type': 'application/json',
  };
}

/** Load a fixture and inject per-test ids into its metadata. */
function fixture(name: string, patch: Record<string, string> = {}): any {
  const raw = JSON.parse(readFileSync(join(FIXTURES, name), 'utf-8'));
  raw.data.metadata = { ...raw.data.metadata, ...patch };
  return raw;
}

/** Deliver a signed webhook body to the real endpoint handler. */
function deliverWebhook(payload: unknown): Promise<Response> {
  const body = JSON.stringify(payload);
  const request = new Request('https://plexiapp.com/api/commerce/webhook/polar', {
    method: 'POST',
    headers: signPolar(body),
    body,
  });
  return webhookPOST({ request } as any) as Promise<Response>;
}

const noCookies = { get: () => undefined } as any;

beforeAll(async () => {
  pg = await startPg();
  process.env.DATABASE_URL = pg.url;
  process.env.PUBLIC_SITE_URL = 'https://plexiapp.com';
  process.env.POLAR_WEBHOOK_SECRET = WEBHOOK_SECRET;
  process.env.SALES_ENABLED = 'true';
  // A predictable checkout backend so we never touch Polar's network.
  setCheckoutClient(async () => ({
    url: 'https://polar.sh/checkout/fake-session',
    checkoutId: 'ch111111-1111-4111-8111-111111111111',
  }));
  // Seed the paid product mapping.
  await query(
    `INSERT INTO app_products (app_id, polar_product_id, price_cents, currency, publisher, artifact_key)
     VALUES ($1, $2, $3, $4, $5, $6)
     ON CONFLICT (app_id) DO NOTHING`,
    [APP_ID, 'p1111111-1111-4111-8111-111111111111', 1200, 'usd', 'plexi', 'paid/reviewed-notes/0.1.0.plexipkg'],
  );
});

afterEach(() => {
  resetArtifactStore();
});

afterAll(async () => {
  resetCheckoutClient();
  await closePool();
  await stopPg(pg);
});

describe('402 envelope + checkout', () => {
  it('formats price and builds an extensible envelope shape', async () => {
    expect(formatPrice(1200, 'usd')).toBe('12.00 USD');
  });

  it('gated download returns a 402 envelope with a checkout option when unowned', async () => {
    const account = await getOrCreateAccount('gate@example.com');
    const { token } = await issueBearerToken(account.id);
    const req = new Request(`https://plexiapp.com/api/registry/artifact/${APP_ID}`, {
      headers: { authorization: `Bearer ${token}` },
    });
    const res = await artifactGET({ request: req, cookies: noCookies, params: { appId: APP_ID } } as any);
    expect(res.status).toBe(402);
    const env = await res.json();
    expect(env.reason).toBe('purchase_required');
    expect(env.price).toBe('12.00 USD');
    expect(env.options[0]).toMatchObject({ type: 'checkout', url: expect.any(String) });
    expect(env.options[0].purchase_id).toBeTruthy();
  });

  it('gated download requires a bearer token', async () => {
    const req = new Request(`https://plexiapp.com/api/registry/artifact/${APP_ID}`);
    const res = await artifactGET({ request: req, cookies: noCookies, params: { appId: APP_ID } } as any);
    expect(res.status).toBe(401);
  });

  it('gated download 404s for a non-paid app', async () => {
    const account = await getOrCreateAccount('free@example.com');
    const { token } = await issueBearerToken(account.id);
    const req = new Request('https://plexiapp.com/api/registry/artifact/calc', {
      headers: { authorization: `Bearer ${token}` },
    });
    const res = await artifactGET({ request: req, cookies: noCookies, params: { appId: 'calc' } } as any);
    expect(res.status).toBe(404);
  });

  it('checkout endpoint returns a checkout url and purchase id', async () => {
    const account = await getOrCreateAccount('checkout@example.com');
    const { token } = await issueBearerToken(account.id);
    const req = new Request('https://plexiapp.com/api/commerce/checkout', {
      method: 'POST',
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body: JSON.stringify({ app_id: APP_ID }),
    });
    const res = await checkoutPOST({ request: req, cookies: noCookies } as any);
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.checkout_url).toContain('polar.sh');
    expect(body.purchase_id).toBeTruthy();
    expect(body.price).toBe('12.00 USD');
  });
});

describe('order webhooks (real signature + real Polar schema)', () => {
  it('order.paid grants entitlement and is idempotent; refund revokes it', async () => {
    const account = await getOrCreateAccount('buyer@example.com');
    const { token } = await issueBearerToken(account.id);

    // Start a checkout to mint a pending purchase row.
    const checkoutReq = new Request('https://plexiapp.com/api/commerce/checkout', {
      method: 'POST',
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body: JSON.stringify({ app_id: APP_ID }),
    });
    const checkoutRes = await checkoutPOST({ request: checkoutReq, cookies: noCookies } as any);
    const { purchase_id } = await checkoutRes.json();
    expect(await getPurchaseState(purchase_id, account.id)).toBe('pending');
    expect(await hasEntitlement(account.id, APP_ID)).toBe(false);

    // Deliver order.paid carrying that purchase_id + account_id in metadata.
    const paid = fixture('order.paid.json', { purchase_id, account_id: account.id });
    // Give this order a unique polar order id for isolation across tests.
    paid.data.id = `order_${purchase_id}`;
    const res = await deliverWebhook(paid);
    expect(res.status).toBe(200);

    expect(await hasEntitlement(account.id, APP_ID)).toBe(true);
    expect(await getPurchaseState(purchase_id, account.id)).toBe('complete');

    // net_cents = net_amount(1200) - platform_fee_amount(110), from the real
    // sandbox-recorded order.paid fixture.
    const row = await query('SELECT net_cents, amount_cents FROM purchases WHERE id = $1', [purchase_id]);
    expect(row.rows[0].net_cents).toBe(1090);
    expect(row.rows[0].amount_cents).toBe(1200);

    // Replay is a no-op — still exactly one row, still complete.
    const replay = await deliverWebhook(paid);
    expect(replay.status).toBe(200);
    const count = await query('SELECT count(*)::int AS n FROM purchases WHERE id = $1', [purchase_id]);
    expect(count.rows[0].n).toBe(1);
    expect(await getPurchaseState(purchase_id, account.id)).toBe('complete');

    // Refund deletes the row; entitlement is gone.
    const refunded = fixture('order.refunded.json', { purchase_id, account_id: account.id });
    refunded.data.id = `order_${purchase_id}`;
    const refundRes = await deliverWebhook(refunded);
    expect(refundRes.status).toBe(200);
    expect(await hasEntitlement(account.id, APP_ID)).toBe(false);
    expect(await getPurchaseState(purchase_id, account.id)).toBeNull();
  });

  it('rejects a webhook with an invalid signature', async () => {
    const paid = fixture('order.paid.json');
    const body = JSON.stringify(paid);
    const headers = signPolar(body);
    headers['webhook-signature'] = 'v1,deadbeefdeadbeefdeadbeefdeadbeef';
    const req = new Request('https://plexiapp.com/api/commerce/webhook/polar', {
      method: 'POST',
      headers,
      body,
    });
    const res = await webhookPOST({ request: req } as any);
    expect(res.status).toBe(401);
  });

  it('skips an order.paid that carries no app purchase metadata', async () => {
    const paid = fixture('order.paid.json');
    paid.data.id = 'order_no_meta';
    paid.data.metadata = {}; // subscription-cycle order shape: no app metadata
    const res = await deliverWebhook(paid);
    expect(res.status).toBe(200);
    const row = await query(`SELECT 1 FROM purchases WHERE polar_order_id = 'order_no_meta'`);
    expect(row.rowCount).toBe(0);
  });
});

describe('subscription webhooks', () => {
  it('subscription.created activates AI Pro; canceled deactivates it', async () => {
    const account = await getOrCreateAccount('subscriber@example.com');
    const sub = fixture('subscription.created.json', { account_id: account.id });
    const res = await deliverWebhook(sub);
    expect(res.status).toBe(200);
    expect(await hasActiveSubscription(account.id)).toBe(true);

    const canceled = fixture('subscription.created.json', { account_id: account.id });
    canceled.type = 'subscription.canceled';
    const cancelRes = await deliverWebhook(canceled);
    expect(cancelRes.status).toBe(200);
    expect(await hasActiveSubscription(account.id)).toBe(false);
  });
});

describe('gated artifact streaming', () => {
  it('streams the paid artifact to an entitled account', async () => {
    const account = await getOrCreateAccount('owner@example.com');
    const { token } = await issueBearerToken(account.id);
    // Grant entitlement directly (a completed purchase).
    await query(
      `INSERT INTO purchases (account_id, app_id, publisher, polar_order_id, amount_cents, net_cents, currency, status)
       VALUES ($1, $2, 'plexi', 'order_owner', 1200, 1140, 'usd', 'complete')`,
      [account.id, APP_ID],
    );
    const bytes = new Uint8Array([0x50, 0x4b, 0x03, 0x04]); // "PK.." — a plausible pkg header
    setArtifactStore({
      async get() {
        return {
          body: new ReadableStream({
            start(controller) {
              controller.enqueue(bytes);
              controller.close();
            },
          }),
          contentLength: bytes.byteLength,
          contentType: 'application/octet-stream',
        };
      },
      async put() {},
    });
    const req = new Request(`https://plexiapp.com/api/registry/artifact/${APP_ID}`, {
      headers: { authorization: `Bearer ${token}` },
    });
    const res = await artifactGET({ request: req, cookies: noCookies, params: { appId: APP_ID } } as any);
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toBe('application/octet-stream');
    const got = new Uint8Array(await res.arrayBuffer());
    expect(Array.from(got)).toEqual(Array.from(bytes));
  });
});

describe('purchase-state endpoint', () => {
  it('scopes purchase reads to the owning account', async () => {
    const owner = await getOrCreateAccount('poll-owner@example.com');
    const other = await getOrCreateAccount('poll-other@example.com');
    const { token: ownerToken } = await issueBearerToken(owner.id);
    const { token: otherToken } = await issueBearerToken(other.id);
    const checkoutReq = new Request('https://plexiapp.com/api/commerce/checkout', {
      method: 'POST',
      headers: { authorization: `Bearer ${ownerToken}`, 'content-type': 'application/json' },
      body: JSON.stringify({ app_id: APP_ID }),
    });
    const { purchase_id } = await (await checkoutPOST({ request: checkoutReq, cookies: noCookies } as any)).json();

    const ownRes = await purchaseGET({
      request: new Request(`https://plexiapp.com/api/commerce/purchase/${purchase_id}`, {
        headers: { authorization: `Bearer ${ownerToken}` },
      }),
      cookies: noCookies,
      params: { id: purchase_id },
    } as any);
    expect(ownRes.status).toBe(200);
    expect((await ownRes.json()).status).toBe('pending');

    const otherRes = await purchaseGET({
      request: new Request(`https://plexiapp.com/api/commerce/purchase/${purchase_id}`, {
        headers: { authorization: `Bearer ${otherToken}` },
      }),
      cookies: noCookies,
      params: { id: purchase_id },
    } as any);
    expect(otherRes.status).toBe(404);
  });
});
