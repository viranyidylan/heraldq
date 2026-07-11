/**************************************
  Per-tenant terminal-row retention
***************************************/

-- How long a tenant's terminal rows are kept before the retention sweeper
-- deletes them. Per-tenant so paid tiers can buy longer history later;
-- defaults apply to every existing and future tenant. DLQ rows are kept
-- longer than successes: they are the ones tenants come back to investigate
-- (and replay), while delivered rows are mostly recent-activity noise.
ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS delivered_retention_days INT NOT NULL DEFAULT 30,
    ADD COLUMN IF NOT EXISTS dlq_retention_days INT NOT NULL DEFAULT 90;

-- The sweeper scans by age; without these it sequential-scans per batch.
-- idx_deliveries_tenant_delivered leads with tenant_id, so it cannot serve
-- an age-only scan.
CREATE INDEX IF NOT EXISTS idx_deliveries_delivered_at
    ON webhook_deliveries (delivered_at);
CREATE INDEX IF NOT EXISTS idx_dlq_abandoned_at
    ON dead_letter_queue (abandoned_at);
