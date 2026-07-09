/**
 * Polar merchant-of-record integration (stint 0339). Polar owns the checkout
 * page, card data, tax/VAT, chargebacks, and payouts; this module only creates
 * checkout sessions and verifies inbound webhooks. Card entry always happens in
 * Polar's hosted browser flow — no payment form ever touches our infrastructure.
 *
 * Webhook shapes and signature verification come from `@polar-sh/sdk` (Polar's
 * own generated schema + Standard Webhooks verification) — we never invent Polar
 * response shapes. Only the thin checkout wrapper below is ours, and it is
 * overridable in tests exactly like the Resend email transport.
 */
import { Polar } from '@polar-sh/sdk';
import { validateEvent, WebhookVerificationError } from '@polar-sh/sdk/webhooks.js';
import { requireEnv, readEnv } from './env';

export { WebhookVerificationError };

/** Revenue split: publishers receive 85% of net (after Polar fees); Plexi keeps 15%. */
export const PUBLISHER_SHARE = 0.85;
export const PLEXI_SHARE = 0.15;

/** The publisher's cut of a net (post-fee) amount, rounded to whole cents. */
export function publisherPayoutCents(netCents: number): number {
  return Math.round(netCents * PUBLISHER_SHARE);
}

/** Which Polar environment to hit. Defaults to sandbox until go-live. */
function polarServer(): 'production' | 'sandbox' {
  return readEnv('POLAR_SERVER') === 'production' ? 'production' : 'sandbox';
}

/** A created checkout session — the URL the buyer opens and Polar's checkout id. */
export interface CheckoutSession {
  url: string;
  checkoutId: string;
}

export interface CheckoutParams {
  productId: string;
  successUrl: string;
  customerEmail: string;
  /** Our account id, threaded to Polar so its customer maps back to our account. */
  externalCustomerId: string;
  /** Echoed back verbatim on every order/subscription webhook for this checkout. */
  metadata: Record<string, string>;
}

/**
 * The checkout backend. Real implementation calls Polar; tests override it so
 * they never hit the network. We own this seam — the never-mock rule applies to
 * Polar's *webhook* shapes (grounded in the SDK schema), not to our own wrapper.
 */
export type CheckoutClient = (params: CheckoutParams) => Promise<CheckoutSession>;

async function livePolarCheckout(params: CheckoutParams): Promise<CheckoutSession> {
  const accessToken = requireEnv(
    'POLAR_ACCESS_TOKEN',
    'the Polar organization access token is required to create checkouts',
  );
  const polar = new Polar({ accessToken, server: polarServer() });
  const checkout = await polar.checkouts.create({
    products: [params.productId],
    successUrl: params.successUrl,
    customerEmail: params.customerEmail,
    externalCustomerId: params.externalCustomerId,
    metadata: params.metadata,
  });
  return { url: checkout.url, checkoutId: checkout.id };
}

let checkoutClient: CheckoutClient | null = null;

/** Override the checkout backend (tests). */
export function setCheckoutClient(c: CheckoutClient): void {
  checkoutClient = c;
}

/** Restore the live Polar checkout backend. */
export function resetCheckoutClient(): void {
  checkoutClient = null;
}

/** Create a Polar checkout session for a product. */
export async function createCheckout(params: CheckoutParams): Promise<CheckoutSession> {
  const client = checkoutClient ?? livePolarCheckout;
  return client(params);
}

/**
 * Verify and parse an inbound Polar webhook. Returns the strongly-typed event
 * (parsed against Polar's generated schema) or throws {@link WebhookVerificationError}
 * on a bad signature. The secret is the Standard Webhooks endpoint secret.
 */
export function verifyPolarWebhook(body: string, headers: Record<string, string>) {
  const secret = requireEnv(
    'POLAR_WEBHOOK_SECRET',
    'the Polar webhook signing secret is required to verify inbound webhooks',
  );
  return validateEvent(body, headers, secret);
}
