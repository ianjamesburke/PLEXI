/**
 * Commerce data layer (stint 0339): products, purchase entitlements, the 402
 * payment-required envelope, subscriptions, and payout accrual. The purchase row
 * IS the entitlement — there is no client-side licensing. A refund deletes the
 * row; the installed app keeps working but stops downloading and updating.
 */
import { getClient, query } from './db';
import { createCheckout, publisherPayoutCents } from './polar';

/** A paid app's Polar product mapping and catalog price. */
export interface AppProduct {
  appId: string;
  polarProductId: string;
  priceCents: number;
  currency: string;
  publisher: string;
  /** Object-storage key of the current paid artifact (null until published). */
  artifactKey: string | null;
}

/** Look up a paid app's product row. Absent → the app is free (or unknown). */
export async function getProduct(appId: string): Promise<AppProduct | null> {
  const res = await query<{
    app_id: string;
    polar_product_id: string;
    price_cents: number;
    currency: string;
    publisher: string;
    artifact_key: string | null;
  }>(
    `SELECT app_id, polar_product_id, price_cents, currency, publisher, artifact_key
     FROM app_products WHERE app_id = $1`,
    [appId],
  );
  if (res.rowCount === 0) return null;
  const r = res.rows[0];
  return {
    appId: r.app_id,
    polarProductId: r.polar_product_id,
    priceCents: r.price_cents,
    currency: r.currency,
    publisher: r.publisher,
    artifactKey: r.artifact_key,
  };
}

/** The Polar product mapping to persist for a first-party paid app (stint 0355). */
export interface AppProductUpsert {
  appId: string;
  polarProductId: string;
  priceCents: number;
  currency: string;
  publisher: string;
}

/**
 * Upsert a paid app's product mapping (stint 0355). The presence of this row is
 * what marks an app PAID. `polar_product_id` is written on first setup and
 * updated in place on later runs (a price change is an update, never a new row);
 * `artifact_key` is left untouched — the publish flow (stint 0344) owns it.
 */
export async function upsertAppProduct(row: AppProductUpsert): Promise<void> {
  await query(
    `INSERT INTO app_products (app_id, polar_product_id, price_cents, currency, publisher)
     VALUES ($1, $2, $3, $4, $5)
     ON CONFLICT (app_id) DO UPDATE
       SET polar_product_id = EXCLUDED.polar_product_id,
           price_cents      = EXCLUDED.price_cents,
           currency         = EXCLUDED.currency,
           publisher        = EXCLUDED.publisher,
           updated_at       = now()`,
    [row.appId, row.polarProductId, row.priceCents, row.currency, row.publisher],
  );
  console.info(
    `[commerce] app_products upserted app_id=${row.appId} product_id=${row.polarProductId} price_cents=${row.priceCents}`,
  );
}

/** Render a cents amount as the envelope's `price` string, e.g. "12.00 USD". */
export function formatPrice(cents: number, currency: string): string {
  return `${(cents / 100).toFixed(2)} ${currency.toUpperCase()}`;
}

/** One rendered option in the extensible 402 envelope. */
export interface CheckoutOption {
  type: 'checkout';
  url: string;
  purchase_id: string;
}

/**
 * The extensible 402 payment-required envelope. Clients render `options`
 * generically and ignore unknown `type`s — this is what keeps a future
 * `{ type: "credits", ... }` option addable at zero client cost.
 */
export interface PaymentRequiredEnvelope {
  reason: 'purchase_required';
  price: string;
  options: CheckoutOption[];
}

export function paymentRequiredEnvelope(
  product: AppProduct,
  checkoutUrl: string,
  purchaseId: string,
): PaymentRequiredEnvelope {
  return {
    reason: 'purchase_required',
    price: formatPrice(product.priceCents, product.currency),
    options: [{ type: 'checkout', url: checkoutUrl, purchase_id: purchaseId }],
  };
}

/**
 * Create a pending purchase row so the client has a `purchase_id` to poll
 * immediately. The order.paid webhook flips it to complete. Returns the new id.
 */
export async function createPendingPurchase(
  accountId: string,
  product: AppProduct,
): Promise<string> {
  const res = await query<{ id: string }>(
    `INSERT INTO purchases
       (account_id, app_id, publisher, amount_cents, currency, status)
     VALUES ($1, $2, $3, $4, $5, 'pending')
     RETURNING id`,
    [accountId, product.appId, product.publisher, product.priceCents, product.currency],
  );
  const id = res.rows[0].id;
  console.info(
    `[commerce] pending purchase created purchase_id=${id} app_id=${product.appId} account_id=${accountId}`,
  );
  return id;
}

/** Attach the Polar checkout id to a pending purchase once the session exists. */
async function setPurchaseCheckoutId(purchaseId: string, checkoutId: string): Promise<void> {
  await query(`UPDATE purchases SET polar_checkout_id = $2, updated_at = now() WHERE id = $1`, [
    purchaseId,
    checkoutId,
  ]);
}

/** The buyer-facing result of starting a checkout. */
export interface StartedCheckout {
  checkoutUrl: string;
  purchaseId: string;
}

/**
 * Start a paid-app checkout: mint a pending purchase row, create the Polar
 * checkout session (metadata is self-contained so the webhook needs no ambient
 * lookup), and record the checkout id. Shared by the checkout endpoint and the
 * gated-download 402 path so a "buy" button and a denied download mint checkouts
 * identically.
 */
export async function startAppCheckout(
  account: { id: string; email: string },
  product: AppProduct,
  successUrl: string,
): Promise<StartedCheckout> {
  const purchaseId = await createPendingPurchase(account.id, product);
  const session = await createCheckout({
    productId: product.polarProductId,
    successUrl,
    customerEmail: account.email,
    externalCustomerId: account.id,
    metadata: {
      purchase_id: purchaseId,
      app_id: product.appId,
      account_id: account.id,
      publisher: product.publisher,
    },
  });
  await setPurchaseCheckoutId(purchaseId, session.checkoutId);
  console.info(
    `[commerce] checkout started purchase_id=${purchaseId} app_id=${product.appId} checkout_id=${session.checkoutId}`,
  );
  return { checkoutUrl: session.url, purchaseId };
}

/** Fields the order.paid webhook carries into the entitlement row. */
export interface PaidOrder {
  polarOrderId: string;
  purchaseId: string | null;
  accountId: string | null;
  appId: string | null;
  publisher: string | null;
  amountCents: number;
  netCents: number;
  currency: string;
}

/**
 * Record a paid order idempotently. If the order id is already recorded, no-op.
 * Otherwise flip the matching pending row (by purchase_id) to complete, or —
 * when no pending row exists (checkout created outside our flow) — insert a
 * complete row from the self-contained webhook metadata.
 */
export async function recordPaidOrder(order: PaidOrder): Promise<void> {
  const client = await getClient();
  try {
    await client.query('BEGIN');

    const dupe = await client.query(
      `SELECT 1 FROM purchases WHERE polar_order_id = $1`,
      [order.polarOrderId],
    );
    if (dupe.rowCount) {
      await client.query('COMMIT');
      console.info(`[commerce] order.paid replay ignored polar_order_id=${order.polarOrderId}`);
      return;
    }

    let updated = 0;
    if (order.purchaseId) {
      const upd = await client.query(
        `UPDATE purchases
           SET status = 'complete', polar_order_id = $2, amount_cents = $3,
               net_cents = $4, currency = $5, refunded_at = NULL, updated_at = now()
         WHERE id = $1 AND status = 'pending'
         RETURNING id`,
        [order.purchaseId, order.polarOrderId, order.amountCents, order.netCents, order.currency],
      );
      updated = upd.rowCount ?? 0;
    }

    if (updated === 0) {
      // No pending row to claim — insert a self-contained complete row. Requires
      // app_id from metadata; without it we cannot attribute the sale.
      if (!order.appId) {
        throw new Error(
          `recordPaidOrder: no pending purchase for order ${order.polarOrderId} and metadata carries no app_id`,
        );
      }
      await client.query(
        `INSERT INTO purchases
           (account_id, app_id, publisher, polar_order_id, amount_cents, net_cents, currency, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'complete')`,
        [
          order.accountId,
          order.appId,
          order.publisher,
          order.polarOrderId,
          order.amountCents,
          order.netCents,
          order.currency,
        ],
      );
    }

    await client.query('COMMIT');
    console.info(
      `[commerce] purchase complete polar_order_id=${order.polarOrderId} app_id=${order.appId ?? '?'} net_cents=${order.netCents} publisher_payout_cents=${publisherPayoutCents(order.netCents)}`,
    );
  } catch (err) {
    await client.query('ROLLBACK');
    console.error(`[commerce] recordPaidOrder failed polar_order_id=${order.polarOrderId}:`, err);
    throw err;
  } finally {
    client.release();
  }
}

/** Delete the purchase row for a refunded order. Idempotent. */
export async function deletePurchaseByOrder(polarOrderId: string): Promise<void> {
  const res = await query(
    `DELETE FROM purchases WHERE polar_order_id = $1 RETURNING app_id`,
    [polarOrderId],
  );
  if (res.rowCount) {
    console.info(
      `[commerce] purchase refunded, row deleted polar_order_id=${polarOrderId} app_id=${res.rows[0].app_id}`,
    );
  }
}

/** True if the account holds a completed, unrefunded purchase for the app. */
export async function hasEntitlement(accountId: string, appId: string): Promise<boolean> {
  const res = await query(
    `SELECT 1 FROM purchases
     WHERE account_id = $1 AND app_id = $2 AND status = 'complete'
     LIMIT 1`,
    [accountId, appId],
  );
  return (res.rowCount ?? 0) > 0;
}

export type PurchaseState = 'pending' | 'complete';

/**
 * Read a purchase's state, scoped to the owning account so one account cannot
 * poll another's purchase. Returns null when unknown or not owned.
 */
export async function getPurchaseState(
  purchaseId: string,
  accountId: string,
): Promise<PurchaseState | null> {
  const res = await query<{ status: PurchaseState }>(
    `SELECT status FROM purchases WHERE id = $1 AND account_id = $2`,
    [purchaseId, accountId],
  );
  return res.rowCount ? res.rows[0].status : null;
}

/** Subscription webhook payload projected onto our columns. */
export interface SubscriptionUpdate {
  polarSubscriptionId: string;
  accountId: string | null;
  status: string;
  tier: string;
  currentPeriodEnd: Date | null;
}

/** Upsert a subscription row, keyed on the Polar subscription id. */
export async function upsertSubscription(sub: SubscriptionUpdate): Promise<void> {
  await query(
    `INSERT INTO subscriptions
       (account_id, polar_subscription_id, status, tier, current_period_end)
     VALUES ($1, $2, $3, $4, $5)
     ON CONFLICT (polar_subscription_id) WHERE polar_subscription_id IS NOT NULL DO UPDATE
       SET status = EXCLUDED.status,
           tier = EXCLUDED.tier,
           current_period_end = EXCLUDED.current_period_end,
           account_id = COALESCE(subscriptions.account_id, EXCLUDED.account_id),
           updated_at = now()`,
    [sub.accountId, sub.polarSubscriptionId, sub.status, sub.tier, sub.currentPeriodEnd],
  );
  console.info(
    `[commerce] subscription upserted polar_subscription_id=${sub.polarSubscriptionId} status=${sub.status} tier=${sub.tier}`,
  );
}

/** Mark a subscription canceled/revoked, keyed on the Polar subscription id. */
export async function markSubscriptionStatus(
  polarSubscriptionId: string,
  status: string,
): Promise<void> {
  const res = await query(
    `UPDATE subscriptions SET status = $2, updated_at = now()
     WHERE polar_subscription_id = $1
     RETURNING id`,
    [polarSubscriptionId, status],
  );
  if (res.rowCount) {
    console.info(
      `[commerce] subscription status=${status} polar_subscription_id=${polarSubscriptionId}`,
    );
  }
}

/** True if the account has an active Plexi AI Pro subscription. */
export async function hasActiveSubscription(accountId: string, tier = 'ai_pro'): Promise<boolean> {
  const res = await query(
    `SELECT 1 FROM subscriptions
     WHERE account_id = $1 AND tier = $2 AND status = 'active'
       AND (current_period_end IS NULL OR current_period_end > now())
     LIMIT 1`,
    [accountId, tier],
  );
  return (res.rowCount ?? 0) > 0;
}
