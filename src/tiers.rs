//! What each account tier buys — the single place tier → enforcement
//! parameters is defined. The pricing page is the contract; these maps are
//! its implementation:
//!
//! | tier          | webhooks/mo            | rate limit | history | DLQ |
//! |---------------|------------------------|------------|---------|-----|
//! | free          | 5,000                  | 240/min    | 1 day   | 7 days |
//! | basic         | 50,000                 | 1,200/min  | 7 days  | 30 days |
//! | professional  | 1,000,000              | 3,000/min  | 30 days | 90 days |
//! | enterprise    | negotiated per tenant  | 3,000/min  | 30 days | 90 days |
//!
//! Enterprise is not a priced tier — it is a negotiated one. A tenant with
//! `tier = enterprise` and no negotiated quota gets exactly the
//! professional limits; the deal is expressed by writing
//! `tenants.monthly_webhook_limit_override` for that tenant (works for any
//! tier — comp a specific professional tenant 2M without inventing a new
//! one). Retention is already per-tenant columns (migration 0009), so
//! enterprise deals can vary those directly.
//!
//! Unknown tiers fall back to the `free` limits — the safe answer when the
//! enum and this map drift is the lowest quota, never the highest.

/// Monthly webhook enqueue cap.
pub fn monthly_webhook_limit(tier: &str) -> i64 {
    match tier {
        "basic" => 50_000,
        // Enterprise defaults to professional: the actual quota comes from
        // the negotiated override, and a not-yet-negotiated enterprise
        // tenant should get the best priced tier's limits, not invented
        // ones.
        "professional" | "enterprise" => 1_000_000,
        _ => 5_000,
    }
}

/// The quota a request should enforce: the tenant's negotiated override
/// when one exists, else the tier default. Overrides are written by hand
/// (or a future deal desk), never by the billing mirror — Paddle sells
/// tiers, humans sell deals.
pub fn effective_monthly_limit(tier: &str, negotiated: Option<i64>) -> i64 {
    negotiated.unwrap_or_else(|| monthly_webhook_limit(tier))
}

/// Sustained per-tenant enqueue rate (requests per second).
pub fn rate_limit_per_second(tier: &str) -> u32 {
    match tier {
        "basic" => 20,        // 1,200/min
        "professional" => 50, // 3,000/min
        "enterprise" => 50,
        _ => 4, // 240/min
    }
}

/// Terminal-row retention in days: `(delivered, dlq)`.
pub fn retention_days(tier: &str) -> (i32, i32) {
    match tier {
        "basic" => (7, 30),
        "professional" | "enterprise" => (30, 90),
        _ => (1, 7),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monthly_limits_match_pricing() {
        assert_eq!(monthly_webhook_limit("free"), 5_000);
        assert_eq!(monthly_webhook_limit("basic"), 50_000);
        assert_eq!(monthly_webhook_limit("professional"), 1_000_000);
        // Enterprise is negotiated; without an override it is professional.
        assert_eq!(
            monthly_webhook_limit("enterprise"),
            monthly_webhook_limit("professional")
        );
    }

    #[test]
    fn rate_limits_match_pricing() {
        assert_eq!(rate_limit_per_second("free") * 60, 240);
        assert_eq!(rate_limit_per_second("basic") * 60, 1_200);
        assert_eq!(rate_limit_per_second("professional") * 60, 3_000);
    }

    #[test]
    fn retention_matches_pricing() {
        assert_eq!(retention_days("free"), (1, 7));
        assert_eq!(retention_days("basic"), (7, 30));
        assert_eq!(retention_days("professional"), (30, 90));
        assert_eq!(retention_days("enterprise"), retention_days("professional"));
    }

    #[test]
    fn negotiated_override_beats_every_default() {
        for tier in ["free", "basic", "professional", "enterprise", "platinum"] {
            assert_eq!(effective_monthly_limit(tier, Some(2_000_000)), 2_000_000);
        }
        // None means "no deal struck" — the tier default applies.
        assert_eq!(effective_monthly_limit("free", None), 5_000);
        assert_eq!(
            effective_monthly_limit("enterprise", None),
            monthly_webhook_limit("professional"),
        );
    }

    #[test]
    fn unknown_tier_falls_back_to_free_not_unlimited() {
        for tier in ["platinum", "", "FREE"] {
            assert_eq!(monthly_webhook_limit(tier), 5_000);
            assert_eq!(rate_limit_per_second(tier), 4);
            assert_eq!(retention_days(tier), (1, 7));
        }
    }
}
