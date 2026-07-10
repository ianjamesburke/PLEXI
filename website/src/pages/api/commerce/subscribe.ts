import type { APIRoute } from 'astro';
import { requireEnv, salesEnabled, siteUrl } from '../../../server/env';
import { createCheckout } from '../../../server/polar';
import { json, requireAccount } from '../../../server/http';

export const prerender = false;

/**
 * Authenticated Plexi AI Pro subscription checkout ($10/mo). Returns the Polar
 * checkout URL; the resulting subscription is written by the subscription
 * webhook, keyed on the account via externalCustomerId. Separate transaction
 * from app purchases — a subscription grants no app licenses and vice versa.
 */
export const POST: APIRoute = async ({ request, cookies }) => {
  try {
    if (!salesEnabled()) {
      return json({ error: 'sales are not enabled yet' }, 403);
    }

    const auth = await requireAccount(request, cookies);
    if (auth instanceof Response) return auth;

    const productId = requireEnv(
      'POLAR_AI_PRO_PRODUCT_ID',
      'the Plexi AI Pro subscription product id is required to start a subscription checkout',
    );
    const successUrl = `${siteUrl()}/account?subscribed=ai_pro`;
    const session = await createCheckout({
      productId,
      successUrl,
      customerEmail: auth.account.email,
      externalCustomerId: auth.account.id,
      metadata: { account_id: auth.account.id, tier: 'ai_pro' },
    });
    console.info(
      `[api/commerce/subscribe] AI Pro checkout created account_id=${auth.account.id} checkout_id=${session.checkoutId}`,
    );
    return json({ checkout_url: session.url }, 200);
  } catch (err) {
    console.error('[api/commerce/subscribe] failed:', err);
    return json({ error: 'server error' }, 500);
  }
};
