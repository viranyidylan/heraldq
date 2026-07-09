-- The dashboard/summary queries filter webhook_deliveries by (tenant_id, delivered_at)
-- on every load; without an index that's a sequential scan over a table that only grows.
-- dead_letter_queue already has idx_dlq_tenant_id, which is sufficient at current scale.
CREATE INDEX IF NOT EXISTS idx_deliveries_tenant_delivered
    ON webhook_deliveries (tenant_id, delivered_at);
