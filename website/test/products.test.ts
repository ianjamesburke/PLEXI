import { afterAll, beforeAll, beforeEach, describe, expect, it } from 'vitest';
import { startPg, stopPg, type PgHandle } from './pg';
import { closePool, query } from '../src/server/db';
import { getProduct } from '../src/server/commerce';
import {
  ensureAppProduct,
  ensureAiProProduct,
  setProductClient,
  resetProductClient,
  type ProductClient,
  type CreateProductArgs,
  type UpdateProductArgs,
} from '../src/server/products';

let pg: PgHandle;

/**
 * An in-memory Polar product catalog standing in for the network. It mirrors the
 * real semantics we depend on: create mints a new id + row, update mutates in
 * place, findByMetadata scans the catalog. Idempotency is a property of our seam,
 * so the stub must let a wrong seam duplicate — it never dedupes for us.
 */
class FakePolar implements ProductClient {
  products = new Map<string, { id: string; name: string; priceCents: number; metadata: Record<string, string> }>();
  createCount = 0;
  updateCount = 0;
  private seq = 0;

  async create(args: CreateProductArgs) {
    this.createCount++;
    const id = `prod_${++this.seq}`;
    this.products.set(id, { id, name: args.name, priceCents: args.priceCents, metadata: args.metadata });
    return { id };
  }
  async update(args: UpdateProductArgs) {
    this.updateCount++;
    const p = this.products.get(args.productId);
    if (!p) throw new Error(`fake update: unknown product ${args.productId}`);
    p.name = args.name;
    p.priceCents = args.priceCents;
    p.metadata = args.metadata;
    return { id: p.id };
  }
  async findByMetadata(key: string, value: string) {
    for (const p of this.products.values()) if (p.metadata[key] === value) return { id: p.id };
    return null;
  }
}

let fake: FakePolar;

beforeAll(async () => {
  pg = await startPg();
  process.env.DATABASE_URL = pg.url;
});

afterAll(async () => {
  resetProductClient();
  await closePool();
  await stopPg(pg);
});

beforeEach(async () => {
  fake = new FakePolar();
  setProductClient(fake);
  await query('DELETE FROM app_products');
});

describe('ensureAppProduct (first-party paid app provisioning)', () => {
  it('creates the Polar product and writes the app_products row on first setup', async () => {
    const res = await ensureAppProduct({
      appId: 'reviewed-notes',
      name: 'Reviewed Notes',
      priceCents: 1200,
      currency: 'USD',
      publisher: 'plexi',
    });
    expect(res.created).toBe(true);
    expect(fake.createCount).toBe(1);

    const row = await getProduct('reviewed-notes');
    expect(row).not.toBeNull();
    expect(row!.polarProductId).toBe(res.productId);
    expect(row!.priceCents).toBe(1200);
    expect(row!.currency).toBe('usd'); // normalized lowercase
    // metadata.app_id is what the order webhook keys off.
    expect(fake.products.get(res.productId)!.metadata.app_id).toBe('reviewed-notes');
  });

  it('is idempotent: a re-run updates the existing product, never duplicates', async () => {
    const first = await ensureAppProduct({
      appId: 'reviewed-notes',
      name: 'Reviewed Notes',
      priceCents: 1200,
      currency: 'usd',
      publisher: 'plexi',
    });
    // Price change → update the same product, keep the same product_id.
    const second = await ensureAppProduct({
      appId: 'reviewed-notes',
      name: 'Reviewed Notes',
      priceCents: 1500,
      currency: 'usd',
      publisher: 'plexi',
    });
    expect(second.created).toBe(false);
    expect(second.productId).toBe(first.productId);
    expect(fake.createCount).toBe(1);
    expect(fake.updateCount).toBe(1);
    expect(fake.products.size).toBe(1);

    const row = await getProduct('reviewed-notes');
    expect(row!.priceCents).toBe(1500);
    // Still exactly one row.
    const count = await query('SELECT count(*)::int AS n FROM app_products WHERE app_id = $1', ['reviewed-notes']);
    expect(count.rows[0].n).toBe(1);
  });

  it('rejects a free/zero price — free apps are never registered', async () => {
    await expect(
      ensureAppProduct({ appId: 'free-app', name: 'Free', priceCents: 0, currency: 'usd', publisher: 'plexi' }),
    ).rejects.toThrow(/positive integer/);
    expect(await getProduct('free-app')).toBeNull();
    expect(fake.createCount).toBe(0);
  });
});

describe('ensureAiProProduct (recurring subscription provisioning)', () => {
  it('creates the recurring product when none exists, then finds it (no duplicate)', async () => {
    const first = await ensureAiProProduct();
    expect(first.created).toBe(true);
    expect(fake.createCount).toBe(1);
    const created = fake.products.get(first.productId)!;
    expect(created.priceCents).toBe(1000); // $10/mo default
    expect(created.metadata.plexi_role).toBe('ai_pro');

    const second = await ensureAiProProduct();
    expect(second.created).toBe(false);
    expect(second.productId).toBe(first.productId);
    expect(fake.createCount).toBe(1); // still only one create
    expect(fake.products.size).toBe(1);
  });
});
