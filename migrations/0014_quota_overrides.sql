/**************************************
  Per-tenant negotiated monthly quota
***************************************/

-- Enterprise (and comped deals of any tier) are negotiated, not priced.
-- A tenant's quota is: this column when set, else the tier default from
-- the tiers module. Written by an operator when a deal is struck — never
-- by the billing mirror: Paddle sells tiers, humans sell deals.
ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS monthly_webhook_limit_override BIGINT;

-- Backfill: nobody has a deal yet, and the column's NULL already means
-- "use the tier default". Exists purely as documentation-in-SQL.
SELECT 1;
