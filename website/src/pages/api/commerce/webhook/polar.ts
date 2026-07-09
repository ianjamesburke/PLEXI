import type { APIRoute } from 'astro';
import {
  deletePurchaseByOrder,
  markSubscriptionStatus,
  recordPaidOrder,
  upsertSubscription,
} from '../../../../server/commerce';
import { verifyPolarWebhook, WebhookVerificationError } from '../../../../server/polar';
import { json } from '../../../../server/http';

export const prerender = false;

/** Read a metadata value as a string, or null. Polar values are string|number|boolean. */
function metaStr(
  metadata: Record<string, string | number | boolean> | undefined,
  key: string,
): string | null {
  const v = metadata?.[key];
  return typeof v === 'string' ? v : v === undefined ? null : String(v);
}

/**
 * Polar webhook receiver. The signature is verified over the RAW body against
 * the Standard Webhooks secret; the parsed event is Polar's own generated type.
 * Idempotent per event: order.paid replays no-op, refunds delete, subscription
 * events upsert. All handlers derive everything from the event payload — nothing
 * is looked up from ambient state.
 */
export const POST: APIRoute = async ({ request }) => {
  const rawBody = await request.text();
  const headers: Record<string, string> = {};
  request.headers.forEach((value, key) => {
    headers[key.toLowerCase()] = value;
  });

  let event;
  try {
    event = verifyPolarWebhook(rawBody, headers);
  } catch (err) {
    if (err instanceof WebhookVerificationError) {
      console.warn('[api/commerce/webhook/polar] signature verification failed');
      return json({ error: 'invalid signature' }, 401);
    }
    console.error('[api/commerce/webhook/polar] failed to parse event:', err);
    return json({ error: 'invalid payload' }, 400);
  }

  try {
    console.info(`[api/commerce/webhook/polar] received type=${event.type}`);
    switch (event.type) {
      case 'order.paid': {
        const order = event.data;
        const appId = metaStr(order.metadata, 'app_id');
        const purchaseId = metaStr(order.metadata, 'purchase_id');
        // Subscription-cycle orders also arrive as order.paid but carry no app
        // purchase metadata — those are handled by the subscription events.
        if (!appId && !purchaseId) {
          console.info('[api/commerce/webhook/polar] order.paid without app metadata — skipped');
          break;
        }
        await recordPaidOrder({
          polarOrderId: order.id,
          purchaseId,
          accountId: metaStr(order.metadata, 'account_id'),
          appId,
          publisher: metaStr(order.metadata, 'publisher'),
          amountCents: order.totalAmount,
          netCents: order.netAmount - order.platformFeeAmount,
          currency: order.currency,
        });
        break;
      }
      case 'order.refunded': {
        await deletePurchaseByOrder(event.data.id);
        break;
      }
      case 'subscription.created':
      case 'subscription.active':
      case 'subscription.updated':
      case 'subscription.uncanceled': {
        const sub = event.data;
        await upsertSubscription({
          polarSubscriptionId: sub.id,
          accountId: metaStr(sub.metadata, 'account_id'),
          status: sub.status,
          tier: metaStr(sub.metadata, 'tier') ?? 'ai_pro',
          currentPeriodEnd: sub.currentPeriodEnd ?? null,
        });
        break;
      }
      case 'subscription.canceled':
      case 'subscription.revoked': {
        const status = event.type === 'subscription.revoked' ? 'revoked' : 'canceled';
        await markSubscriptionStatus(event.data.id, status);
        break;
      }
      default:
        // Unhandled event types are acknowledged so Polar stops retrying.
        console.info(`[api/commerce/webhook/polar] unhandled type=${event.type}`);
    }
    return json({ received: true }, 200);
  } catch (err) {
    console.error(`[api/commerce/webhook/polar] handler failed type=${event.type}:`, err);
    return json({ error: 'server error' }, 500);
  }
};
