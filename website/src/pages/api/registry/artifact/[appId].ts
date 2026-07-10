import type { APIRoute } from 'astro';
import {
  getProduct,
  hasEntitlement,
  paymentRequiredEnvelope,
  startAppCheckout,
} from '../../../../server/commerce';
import { salesEnabled, siteUrl } from '../../../../server/env';
import { getArtifact } from '../../../../server/storage';
import { json, requireAccount } from '../../../../server/http';

export const prerender = false;

/**
 * Gated paid-artifact download. The host presents its account bearer token; the
 * server decides. This is the primary anti-fork lever — it gates BOTH the
 * initial install and every update. Free apps are NOT served here (they stay
 * static under /registry/v1/packages/); this route is paid-only.
 *
 * - No bearer          → 401 (log in first).
 * - Bearer, no purchase → 402 with the extensible envelope (checkout url + id).
 * - Bearer, purchased   → 200 streaming the .plexipkg from private storage.
 *
 * Registry METADATA stays free (marketplace-hosted.md §1); only the bytes gate.
 */
export const GET: APIRoute = async ({ request, cookies, params }) => {
  const appId = params.appId ?? '';
  try {
    if (!appId) return json({ error: 'app id is required' }, 400);

    const product = await getProduct(appId);
    // Not a paid app → this gated route does not serve it. Free artifacts are
    // static files; a 404 keeps the paid/free split unambiguous.
    if (!product) return json({ error: `no paid product for app '${appId}'` }, 404);

    const auth = await requireAccount(request, cookies);
    if (auth instanceof Response) return auth;

    if (await hasEntitlement(auth.account.id, appId)) {
      if (!product.artifactKey) {
        console.error(`[api/registry/artifact] entitled but no artifact_key app_id=${appId}`);
        return json({ error: 'artifact not available' }, 404);
      }
      const artifact = await getArtifact(product.artifactKey);
      if (!artifact) {
        console.error(
          `[api/registry/artifact] artifact_key missing in storage app_id=${appId} key=${product.artifactKey}`,
        );
        return json({ error: 'artifact not available' }, 404);
      }
      console.info(
        `[api/registry/artifact] streaming paid artifact app_id=${appId} account_id=${auth.account.id}`,
      );
      const responseHeaders: Record<string, string> = {
        'content-type': artifact.contentType,
        'content-disposition': `attachment; filename="${appId}.plexipkg"`,
        'cache-control': 'private, no-store',
      };
      if (artifact.contentLength !== undefined) {
        responseHeaders['content-length'] = String(artifact.contentLength);
      }
      return new Response(artifact.body, { status: 200, headers: responseHeaders });
    }

    // No entitlement. If sales are live, mint a checkout so the client can buy;
    // otherwise the envelope has no purchasable option yet.
    if (!salesEnabled()) {
      console.info(`[api/registry/artifact] payment required, sales disabled app_id=${appId}`);
      return json(
        { reason: 'purchase_required', price: `${(product.priceCents / 100).toFixed(2)} ${product.currency.toUpperCase()}`, options: [] },
        402,
      );
    }

    const successUrl = `${siteUrl()}/account?purchased=${encodeURIComponent(appId)}`;
    const started = await startAppCheckout(auth.account, product, successUrl);
    console.info(
      `[api/registry/artifact] payment required app_id=${appId} account_id=${auth.account.id} purchase_id=${started.purchaseId}`,
    );
    return json(paymentRequiredEnvelope(product, started.checkoutUrl, started.purchaseId), 402);
  } catch (err) {
    console.error(`[api/registry/artifact] failed app_id=${appId}:`, err);
    return json({ error: 'server error' }, 500);
  }
};
