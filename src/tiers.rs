//! What each account tier buys — the single place tier → enforcement
//! parameters is defined. The pricing page is the contract; these maps are
//! its implementation:
//!
//! | tier          | webhooks/mo | rate limit | history retention | DLQ retention |
//! |---------------|-------------|------------|------------------|---------------|
//! | free          | 5,000       | 240/min    | 1 day            | 7 days        |
//! | basic         | 50,000      | 1,200/min  | 7 days           | 30 days       |
//! | professional  | 500,000     | 3,000/min  | 30 days          | 90 days       |
//! | enterprise    | 5,000,000   | 3,000/min  | 90 days          | 365 days      |
//!
//! Unknown tiers fall back to the `free` limits — the safe answer when the
//! enum and this map drift is the lowest quota, never the highest.

/// Monthly webhook enqueue cap.
pub fn monthly_webhook_limit(tier: &str) -> i64 {
    match tier {
        "basic" => 50_000,
        "professional" => 500_000,
        "enterprise" => 5_000_000,
        _ => 5_000,
    }
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
        "professional" => (30, 90),
        "enterprise" => (90, 365),
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
        assert_eq!(monthly_webhook_limit("professional"), 500_000);
        assert_eq!(monthly_webhook_limit("enterprise"), 5_000_000);
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
