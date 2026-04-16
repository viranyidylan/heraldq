# Implement HMAC payload signing the type-system-friendly way

## Context


## Approach plan

### 1. 


### 2. Split the summary into small compile-checked queries, compose concurrently


## Files to touch



## Verification (walking skeleton: test after each small step)

1. After newtype: `cargo check` — enqueue still compiles, no behaviour change.
2. After one split query (e.g. `count_delivered_today`): call it directly from a scratch test or curl `/summary` — numbers still match pre-refactor for one tenant.
3. After full `try_join!` swap: seed two tenants with jobs, hit `/summary` with each tenant's API key, confirm `queued` differs per tenant (this is the bug repro → fix proof).
4. `cargo sqlx prepare` then `cargo check --offline` to confirm offline build still works (CI relies on it).
5. Manual: `curl -H "x-api-key: …" localhost:PORT/summary` for tenant A and tenant B — `queued` must be tenant-scoped.