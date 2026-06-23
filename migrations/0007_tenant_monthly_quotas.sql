/**************************************
  Per-tenant monthly webhook quota
***************************************/

-- One row per tenant per calendar month. month_start is the first day of the
-- month (DATE_TRUNC('month', NOW())::date), giving a stable, readable key.
CREATE TABLE IF NOT EXISTS tenant_monthly_quotas (
    tenant_id     UUID    NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    month_start   DATE    NOT NULL,
    webhook_count BIGINT  NOT NULL DEFAULT 0,
    PRIMARY KEY (tenant_id, month_start)
);
