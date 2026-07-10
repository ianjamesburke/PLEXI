/**
 * First-party product provisioning (stint 0355). Plexi sells its own paid apps
 * and the AI Pro subscription under Plexi's own Polar account, so Plexi is the
 * seller of record — no publisher onboarding, payout rail, or review queue. This
 * module is the create-or-update seam that mints the Polar `product_id` the
 * checkout flow (stint 0339) assumes but nothing else creates.
 *
 * Polar owns the product catalog; we own the mapping. `ensureAppProduct` is
 * idempotent on the `app_products` row: first call creates the Polar product and
 * writes the row; later calls PATCH the existing product (a price change is an
 * update, never a duplicate). `ensureAiProProduct` is idempotent on Polar
 * product metadata (`plexi_role=ai_pro`) since the subscription has no app row.
 *
 * The Polar calls live behind a {@link ProductClient} seam, overridable in tests
 * exactly like the checkout and artifact-store seams — the never-mock rule
 * applies to Polar's response shapes (grounded in the live sandbox), not to this
 * thin wrapper. This is the shared seam the third-party path (stint 0344) reuses.
 */
import { Polar } from '@polar-sh/sdk';
import type { PresentmentCurrency } from '@polar-sh/sdk/models/components/presentmentcurrency.js';
import { requireEnv, readEnv } from './env';
import { getProduct, upsertAppProduct } from './commerce';

/** Metadata key marking the single recurring Plexi AI Pro subscription product. */
export const AI_PRO_ROLE = 'ai_pro';
/** Default catalog price of Plexi AI Pro: $10.00/mo. */
export const AI_PRO_PRICE_CENTS = 1000;

/** Which Polar environment to hit. Defaults to sandbox until go-live. */
function polarServer(): 'production' | 'sandbox' {
  return readEnv('POLAR_SERVER') === 'production' ? 'production' : 'sandbox';
}

/** A created or updated Polar product — just the id we persist. */
export interface PolarProductRef {
  id: string;
}

/** Arguments for creating a Polar product. `recurringInterval` null = one-time. */
export interface CreateProductArgs {
  name: string;
  priceCents: number;
  currency: string;
  recurringInterval: 'month' | 'year' | null;
  metadata: Record<string, string>;
}

/** Arguments for updating an existing Polar product (price/name/metadata). */
export interface UpdateProductArgs {
  productId: string;
  name: string;
  priceCents: number;
  currency: string;
  metadata: Record<string, string>;
}

/**
 * The Polar product backend. Real implementation calls Polar; tests override it
 * so they never hit the network. We own this seam — the never-mock rule applies
 * to Polar's *response* shapes, not to our own wrapper.
 */
export interface ProductClient {
  create(args: CreateProductArgs): Promise<PolarProductRef>;
  update(args: UpdateProductArgs): Promise<PolarProductRef>;
  /** Find a product by an exact metadata key/value, or null. Used for AI Pro. */
  findByMetadata(key: string, value: string): Promise<PolarProductRef | null>;
}

function polar(): Polar {
  const accessToken = requireEnv(
    'POLAR_ACCESS_TOKEN',
    'the Polar organization access token is required to create products',
  );
  return new Polar({ accessToken, server: polarServer() });
}

/**
 * Live Polar product client. With an *organization* access token, Polar infers
 * the organization and rejects an explicit `organizationId` — so we omit it
 * (verified against the live sandbox). Prices are set as a single fixed price;
 * an update replaces the price array, archiving the prior price.
 */
const livePolarProducts: ProductClient = {
  async create(args: CreateProductArgs): Promise<PolarProductRef> {
    const product = await polar().products.create({
      name: args.name,
      recurringInterval: args.recurringInterval,
      prices: [
        {
          amountType: 'fixed',
          priceAmount: args.priceCents,
          priceCurrency: args.currency as PresentmentCurrency,
        },
      ],
      metadata: args.metadata,
    });
    return { id: product.id };
  },
  async update(args: UpdateProductArgs): Promise<PolarProductRef> {
    const product = await polar().products.update({
      id: args.productId,
      productUpdate: {
        name: args.name,
        metadata: args.metadata,
        prices: [
          {
            amountType: 'fixed',
            priceAmount: args.priceCents,
            priceCurrency: args.currency as PresentmentCurrency,
          },
        ],
      },
    });
    return { id: product.id };
  },
  async findByMetadata(key: string, value: string): Promise<PolarProductRef | null> {
    // The Plexi org has a small product catalog; one page covers it. Filtering
    // is client-side because Polar's list API does not filter on metadata.
    const list = await polar().products.list({ limit: 100 });
    for (const item of list.result?.items ?? []) {
      if ((item.metadata as Record<string, unknown> | undefined)?.[key] === value) {
        return { id: item.id };
      }
    }
    return null;
  },
};

let productClient: ProductClient | null = null;

/** Override the Polar product backend (tests). */
export function setProductClient(c: ProductClient): void {
  productClient = c;
}

/** Restore the live Polar product backend. */
export function resetProductClient(): void {
  productClient = null;
}

function client(): ProductClient {
  return productClient ?? livePolarProducts;
}

/** Input describing a first-party paid app to provision. */
export interface AppProductInput {
  appId: string;
  /** Display name shown on the Polar checkout page. */
  name: string;
  priceCents: number;
  currency: string;
  publisher: string;
}

/** Result of provisioning: the Polar product id and whether it was newly created. */
export interface EnsuredProduct {
  productId: string;
  created: boolean;
}

/**
 * Provision the Polar product for a first-party paid app, idempotently. First
 * call creates the product (metadata carries `app_id` so the order webhook can
 * attribute the sale) and writes the `app_products` row; later calls PATCH the
 * existing product and update the row. The `product_id` is written once and
 * never changes. Free apps are never passed here — the caller enforces a price.
 */
export async function ensureAppProduct(input: AppProductInput): Promise<EnsuredProduct> {
  if (!Number.isInteger(input.priceCents) || input.priceCents <= 0) {
    throw new Error(
      `ensureAppProduct: price_cents must be a positive integer (free apps are not registered); got ${input.priceCents}`,
    );
  }
  const currency = input.currency.toLowerCase();
  const metadata = { app_id: input.appId, plexi_kind: 'app' };
  const existing = await getProduct(input.appId);

  if (existing) {
    const updated = await client().update({
      productId: existing.polarProductId,
      name: input.name,
      priceCents: input.priceCents,
      currency,
      metadata,
    });
    await upsertAppProduct({
      appId: input.appId,
      polarProductId: updated.id,
      priceCents: input.priceCents,
      currency,
      publisher: input.publisher,
    });
    console.info(
      `[products] app product updated app_id=${input.appId} product_id=${updated.id} price_cents=${input.priceCents}`,
    );
    return { productId: updated.id, created: false };
  }

  const created = await client().create({
    name: input.name,
    priceCents: input.priceCents,
    currency,
    recurringInterval: null,
    metadata,
  });
  await upsertAppProduct({
    appId: input.appId,
    polarProductId: created.id,
    priceCents: input.priceCents,
    currency,
    publisher: input.publisher,
  });
  console.info(
    `[products] app product created app_id=${input.appId} product_id=${created.id} price_cents=${input.priceCents}`,
  );
  return { productId: created.id, created: true };
}

/** Input for provisioning the Plexi AI Pro subscription product. */
export interface AiProInput {
  priceCents?: number;
  currency?: string;
}

/**
 * Ensure the recurring Plexi AI Pro subscription product exists, idempotently.
 * Idempotency is keyed on Polar product metadata (`plexi_role=ai_pro`) because a
 * subscription has no `app_products` row. Returns the product id the operator
 * wires into `/api/commerce/subscribe` via `POLAR_AI_PRO_PRODUCT_ID`. Re-running
 * finds the existing product and returns the same id — it never duplicates.
 */
export async function ensureAiProProduct(input: AiProInput = {}): Promise<EnsuredProduct> {
  const priceCents = input.priceCents ?? AI_PRO_PRICE_CENTS;
  const currency = (input.currency ?? 'usd').toLowerCase();
  const metadata = { plexi_role: AI_PRO_ROLE };

  const found = await client().findByMetadata('plexi_role', AI_PRO_ROLE);
  if (found) {
    const updated = await client().update({
      productId: found.id,
      name: 'Plexi AI Pro',
      priceCents,
      currency,
      metadata,
    });
    console.info(`[products] AI Pro product present product_id=${updated.id} price_cents=${priceCents}`);
    return { productId: updated.id, created: false };
  }

  const created = await client().create({
    name: 'Plexi AI Pro',
    priceCents,
    currency,
    recurringInterval: 'month',
    metadata,
  });
  console.info(`[products] AI Pro product created product_id=${created.id} price_cents=${priceCents}`);
  return { productId: created.id, created: true };
}
