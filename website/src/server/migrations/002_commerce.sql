-- Commerce schema for the Polar money path (stint 0339). Fills the placeholder
-- tables declared in 001_init.sql with the columns the checkout, webhook, and
-- gated-download flows actually need. Applied once, tracked in schema_migrations.

-- Maps a marketplace app id to its Polar product. Presence of a row means the
-- app is PAID (free apps are never listed here); the checkout endpoint and the
-- gated-download endpoint both key off this table. price_cents/currency are the
-- catalog price shown in the 402 envelope; polar_product_id drives checkout.
CREATE TABLE IF NOT EXISTS app_products (
  app_id           TEXT PRIMARY KEY,
  polar_product_id TEXT NOT NULL,
  price_cents      INTEGER NOT NULL,
  currency         TEXT NOT NULL DEFAULT 'usd',
  publisher        TEXT NOT NULL,
  -- Object-storage key for the current paid artifact (private Railway bucket).
  -- Set at publish time (stint 0344); the gated endpoint streams this key.
  artifact_key     TEXT,
  created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- A purchase row is the entitlement. A row is created 'pending' at checkout time
-- (so the client has a purchase_id to poll immediately) and flipped to
-- 'complete' by the order.paid webhook. A refund DELETES the row entirely — the
-- installed code keeps working, but downloads/updates stop.
ALTER TABLE purchases
  ADD COLUMN IF NOT EXISTS status            TEXT NOT NULL DEFAULT 'pending',
  ADD COLUMN IF NOT EXISTS polar_checkout_id TEXT,
  ADD COLUMN IF NOT EXISTS net_cents         INTEGER,
  ADD COLUMN IF NOT EXISTS publisher         TEXT,
  ADD COLUMN IF NOT EXISTS updated_at        TIMESTAMPTZ NOT NULL DEFAULT now();

-- Webhook idempotency: an order.paid replay must not double-write. polar_order_id
-- is NULL on pending rows (Postgres treats NULLs as distinct, so many pending
-- rows coexist) and unique once the order lands.
CREATE UNIQUE INDEX IF NOT EXISTS idx_purchases_polar_order_id
  ON purchases(polar_order_id) WHERE polar_order_id IS NOT NULL;

-- Entitlement lookups: "does this account own this app?" and payout accrual.
CREATE INDEX IF NOT EXISTS idx_purchases_account_app ON purchases(account_id, app_id);
CREATE INDEX IF NOT EXISTS idx_purchases_publisher   ON purchases(publisher);

-- Subscriptions: one Polar subscription id maps to one row; the webhook upserts.
ALTER TABLE subscriptions
  ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
CREATE UNIQUE INDEX IF NOT EXISTS idx_subscriptions_polar_id
  ON subscriptions(polar_subscription_id) WHERE polar_subscription_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_subscriptions_account ON subscriptions(account_id);

-- Payouts ledger: every operator-run monthly transfer to a publisher. Payout is
-- 85% of net (after Polar fees). One row per publisher per period per transfer.
CREATE TABLE IF NOT EXISTS payouts (
  id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  publisher      TEXT NOT NULL,
  period_start   DATE NOT NULL,
  period_end     DATE NOT NULL,
  gross_cents    INTEGER NOT NULL,
  net_cents      INTEGER NOT NULL,
  payout_cents   INTEGER NOT NULL,
  currency       TEXT NOT NULL DEFAULT 'usd',
  transferred_at TIMESTAMPTZ,
  note           TEXT,
  created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_payouts_publisher ON payouts(publisher);
