-- Rename package identities without rewriting values or changing entitlements.
-- The migration runner executes this once inside its advisory-locked transaction.
ALTER TABLE app_products RENAME TO plugin_products;
ALTER TABLE plugin_products RENAME COLUMN app_id TO plugin_id;
ALTER TABLE purchases RENAME COLUMN app_id TO plugin_id;
ALTER INDEX idx_purchases_account_app RENAME TO idx_purchases_account_plugin;
