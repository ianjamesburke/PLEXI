import type { APIRoute } from 'astro';
import { getPurchaseState } from '../../../../server/commerce';
import { json, requireAccount } from '../../../../server/http';

export const prerender = false;

/**
 * Authenticated purchase-state read. The marketplace app and CLI poll this after
 * starting a checkout: pending until the order.paid webhook lands, then complete.
 * Scoped to the owning account so one account cannot poll another's purchase.
 */
export const GET: APIRoute = async ({ request, cookies, params }) => {
  try {
    const auth = await requireAccount(request, cookies);
    if (auth instanceof Response) return auth;

    const purchaseId = params.id ?? '';
    if (!purchaseId) return json({ error: 'purchase id is required' }, 400);

    const state = await getPurchaseState(purchaseId, auth.account.id);
    if (state === null) return json({ error: 'purchase not found' }, 404);

    return json({ purchase_id: purchaseId, status: state }, 200);
  } catch (err) {
    console.error(`[api/commerce/purchase] failed for id="${params.id}":`, err);
    return json({ error: 'server error' }, 500);
  }
};
