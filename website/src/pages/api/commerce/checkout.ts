import type { APIRoute } from 'astro';
import { getProduct, startAppCheckout } from '../../../server/commerce';
import { salesEnabled, siteUrl } from '../../../server/env';
import { json, parseJsonBody, requireAccount } from '../../../server/http';

export const prerender = false;

/**
 * Authenticated checkout creation. Body: { app_id }. Returns the Polar checkout
 * URL and a purchase_id the caller polls. The marketplace app "buy" button and
 * the CLI both hit this. Free/unknown apps have no product and 404.
 */
export const POST: APIRoute = async ({ request, cookies }) => {
  try {
    if (!salesEnabled()) {
      return json({ error: 'sales are not enabled yet' }, 403);
    }

    const auth = await requireAccount(request, cookies);
    if (auth instanceof Response) return auth;

    const body = await parseJsonBody(request);
    if (body instanceof Response) return body;
    const appId = typeof body.app_id === 'string' ? body.app_id.trim() : '';
    if (!appId) return json({ error: 'app_id is required' }, 400);

    const product = await getProduct(appId);
    if (!product) return json({ error: `no paid product for app '${appId}'` }, 404);

    const successUrl = `${siteUrl()}/account?purchased=${encodeURIComponent(appId)}`;
    const started = await startAppCheckout(auth.account, product, successUrl);
    console.info(
      `[api/commerce/checkout] checkout created app_id=${appId} account_id=${auth.account.id} purchase_id=${started.purchaseId}`,
    );
    return json(
      {
        checkout_url: started.checkoutUrl,
        purchase_id: started.purchaseId,
        price: `${(product.priceCents / 100).toFixed(2)} ${product.currency.toUpperCase()}`,
      },
      200,
    );
  } catch (err) {
    console.error('[api/commerce/checkout] failed:', err);
    return json({ error: 'server error' }, 500);
  }
};
