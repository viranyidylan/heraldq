/**************************************
  Paystack billing sync (webhook mirror)
***************************************/

-- Mirror of Paystack customer state, kept current by /webhooks/paystack.
-- `tenant_id` is the bridge to our own accounts: matched lazily by email,
-- because a Paystack customer can exist before (or without) a heraldQ tenant.
CREATE TABLE IF NOT EXISTS paystack_customers (
    paystack_customer_code TEXT PRIMARY KEY,      -- "CUS_..."
    email                  TEXT NOT NULL,
    tenant_id              UUID REFERENCES tenants(id) ON DELETE SET NULL,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS paystack_customers_email_idx ON paystack_customers(email);

-- Mirror of Paystack subscription state. `tier` is derived from the plan code
-- at event time (the mapping lives in code, since Paystack plans carry no
-- metadata). `email_token` is stored because Paystack needs it to cancel a
-- subscription programmatically later.
CREATE TABLE IF NOT EXISTS paystack_subscriptions (
    paystack_subscription_code TEXT PRIMARY KEY,  -- "SUB_..."
    paystack_customer_code     TEXT NOT NULL REFERENCES paystack_customers(paystack_customer_code) ON DELETE CASCADE,
    status                     TEXT NOT NULL,     -- active | non_renewing | disabled
    plan_code                  TEXT NOT NULL,
    tier                       TEXT,              -- derived: basic | professional
    email_token                TEXT,
    amount                     BIGINT,            -- kobo/cents, as delivered
    currency                   TEXT,
    created_at                 TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at                 TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS paystack_subscriptions_customer_idx
    ON paystack_subscriptions(paystack_customer_code);

-- Dedup ledger. Paystack, unlike Paddle, provides no event id and signs with
-- an untimestamped HMAC — idempotency is entirely on our side. The key is
-- synthesized per event type (charges use data.reference, entities use
-- data.id / data.subscription_code combined with the event name).
CREATE TABLE IF NOT EXISTS paystack_processed_events (
    dedup_key    TEXT PRIMARY KEY,                -- "<event>:<reference|id|code>"
    event_type   TEXT NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
