import { readFileSync } from 'node:fs';
import pg from 'pg';
import { expect, it } from 'vitest';
import { startPg, stopPg } from './pg';
import { closePool, ensureMigrated, query } from '../src/server/db';
import { hasEntitlement } from '../src/server/commerce';

it('preserves 0.4.x product identities and paid entitlements through the plugin rename', async () => {
  const database = await startPg();
  process.env.DATABASE_URL = database.url;
  const client = new pg.Client({ connectionString: database.url });
  try {
    await client.connect();
    await client.query('CREATE TABLE schema_migrations (filename TEXT PRIMARY KEY, applied_at TIMESTAMPTZ DEFAULT now())');
    for (const filename of ['001_init.sql', '002_commerce.sql']) {
      await client.query(readFileSync(new URL(`../src/server/migrations/${filename}`, import.meta.url), 'utf8'));
      await client.query('INSERT INTO schema_migrations (filename) VALUES ($1)', [filename]);
    }
    const account = await client.query("INSERT INTO accounts (email) VALUES ('migration@example.com') RETURNING id");
    const accountId = account.rows[0].id;
    await client.query("INSERT INTO app_products (app_id, polar_product_id, price_cents, publisher) VALUES ('legacy', 'polar-existing', 1200, 'plexi')");
    await client.query("INSERT INTO purchases (account_id, app_id, status, polar_order_id) VALUES ($1, 'legacy', 'complete', 'order-existing')", [accountId]);
    await ensureMigrated();
    await ensureMigrated();
    expect(await hasEntitlement(accountId, 'legacy')).toBe(true);
    const product = await query("SELECT plugin_id, polar_product_id FROM plugin_products WHERE plugin_id = 'legacy'");
    expect(product.rows).toEqual([{ plugin_id: 'legacy', polar_product_id: 'polar-existing' }]);
    const receipt = await query("SELECT 1 FROM schema_migrations WHERE filename = '003_plugin_names.sql'");
    expect(receipt.rowCount).toBe(1);
  } finally {
    await client.end();
    await closePool();
    await stopPg(database);
  }
});
