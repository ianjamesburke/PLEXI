-- Initial account-service schema. Applied once, tracked in schema_migrations.
-- gen_random_uuid() is built into PostgreSQL 13+; no pgcrypto extension needed.

CREATE TABLE IF NOT EXISTS subscribers (
  email      TEXT PRIMARY KEY,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  source     TEXT
);

CREATE TABLE IF NOT EXISTS accounts (
  id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  email      TEXT UNIQUE NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Opaque bearer tokens, stored as SHA-256 hex hashes. Revocable via revoked_at.
CREATE TABLE IF NOT EXISTS auth_tokens (
  token_hash TEXT PRIMARY KEY,
  account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  issued_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  revoked_at TIMESTAMPTZ
);

-- CLI device-code flow. status: pending | approved | consumed | expired.
CREATE TABLE IF NOT EXISTS device_codes (
  code            TEXT PRIMARY KEY,
  poll_token_hash TEXT NOT NULL,
  email           TEXT,
  account_id      UUID REFERENCES accounts(id) ON DELETE SET NULL,
  status          TEXT NOT NULL DEFAULT 'pending',
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at      TIMESTAMPTZ NOT NULL
);

-- Single-use, purpose-tagged magic links. purpose: login | device_approve | account_delete.
CREATE TABLE IF NOT EXISTS magic_link_tokens (
  token_hash  TEXT PRIMARY KEY,
  email       TEXT NOT NULL,
  purpose     TEXT NOT NULL,
  device_code TEXT REFERENCES device_codes(code) ON DELETE SET NULL,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at  TIMESTAMPTZ NOT NULL,
  used_at     TIMESTAMPTZ
);

-- Placeholder commerce tables (0339 fills them). Defined here so migrations
-- stay linear. account_id is nullable on purchases so account deletion can
-- anonymize the row rather than destroying the sales record.
CREATE TABLE IF NOT EXISTS purchases (
  id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  account_id     UUID REFERENCES accounts(id) ON DELETE SET NULL,
  app_id         TEXT NOT NULL,
  polar_order_id TEXT,
  amount_cents   INTEGER,
  currency       TEXT,
  refunded_at    TIMESTAMPTZ,
  created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS subscriptions (
  id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  account_id            UUID REFERENCES accounts(id) ON DELETE CASCADE,
  polar_subscription_id TEXT,
  status                TEXT,
  tier                  TEXT,
  current_period_end    TIMESTAMPTZ,
  created_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS publishers (
  id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  account_id     UUID REFERENCES accounts(id) ON DELETE CASCADE,
  display_name   TEXT,
  payout_details TEXT,
  tax_info       TEXT,
  created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Lookup indexes for account-scoped deletes and email-scoped auth queries.
CREATE INDEX IF NOT EXISTS idx_purchases_account_id ON purchases(account_id);
CREATE INDEX IF NOT EXISTS idx_device_codes_account_id ON device_codes(account_id);
CREATE INDEX IF NOT EXISTS idx_device_codes_email ON device_codes(email);
CREATE INDEX IF NOT EXISTS idx_magic_link_tokens_email ON magic_link_tokens(email);
