TRUNCATE tenants CASCADE;

ALTER TABLE tenants 
    ADD COLUMN email TEXT UNIQUE NOT NULL,
    ADD COLUMN hmac_secret TEXT NOT NULL;

CREATE TABLE IF NOT EXISTS beta_codes (
    code                    TEXT PRIMARY KEY, --friendly key like BETA_XYXZ
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    redeemed_at             TIMESTAMPTZ,
    redeemed_by_tenant_id   UUID REFERENCES tenants(id),
    expires_at              TIMESTAMPTZ,
    CONSTRAINT redemption_consistency
        CHECK ((redeemed_at IS NULL) = (redeemed_by_tenant_id IS NULL))
);
