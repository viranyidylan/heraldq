/**************************************
  Paddle billing sync (webhook mirror)
***************************************/

-- Mirror of Paddle customer state, kept current by /webhooks/paddle.
-- `tenant_id` is the bridge to our own accounts: matched lazily by email,
-- because a Paddle customer can exist before (or without) a heraldQ tenant.
CREATE TABLE IF NOT EXISTS paddle_customers (
    paddle_customer_id TEXT PRIMARY KEY,            -- "ctm_01h..."
    email              TEXT NOT NULL,
    tenant_id          UUID REFERENCES tenants(id) ON DELETE SET NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS paddle_customers_email_idx ON paddle_customers(email);

-- Mirror of Paddle subscription state. `tier` mirrors the price's
-- custom_data.tier verbatim as TEXT (it's metadata we control, but the
-- mirror must not error on a value the account_tier enum doesn't know —
-- interpretation happens in code, at read time). scheduled_change is raw
-- JSONB ({action, effective_at}) since only its presence matters for gating.
CREATE TABLE IF NOT EXISTS paddle_subscriptions (
    paddle_subscription_id TEXT PRIMARY KEY,        -- "sub_01h..."
    paddle_customer_id     TEXT NOT NULL REFERENCES paddle_customers(paddle_customer_id) ON DELETE CASCADE,
    status                 TEXT NOT NULL,           -- active | trialing | past_due | paused | canceled
    tier                   TEXT,                    -- price custom_data.tier, mirrored verbatim
    price_id               TEXT NOT NULL,
    product_id             TEXT NOT NULL,
    scheduled_change       JSONB,
    current_period_start   TIMESTAMPTZ,
    current_period_end     TIMESTAMPTZ,
    canceled_at            TIMESTAMPTZ,
    next_billed_at         TIMESTAMPTZ,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS paddle_subscriptions_customer_idx
    ON paddle_subscriptions(paddle_customer_id);

-- Dedup ledger: Paddle delivers at-least-once with the same event_id on every
-- retry. Insert-first — a unique violation means "already processed, ack".
CREATE TABLE IF NOT EXISTS paddle_processed_events (
    event_id     TEXT PRIMARY KEY,                  -- "evt_01h..."
    event_type   TEXT NOT NULL,
    occurred_at  TIMESTAMPTZ,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
