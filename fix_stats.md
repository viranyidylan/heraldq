# Fix tenant-scoped stats the type-system-friendly way

## Context

The bug: [src/db/stats.rs:30-44](src/db/stats.rs#L30-L44) — `webhook_activity_summary` takes a `tenant_id` but the `graphile_worker.jobs` subquery **never filters by `queue_name`**, so the `queued` count leaks every tenant's jobs into every tenant's dashboard. Meanwhile [src/db/stats.rs:4-15](src/db/stats.rs#L4-L15) (`count_jobs_queued_today`) already does this correctly with `queue_name = $1`.

Root cause isn't just a missing WHERE clause — it's that the **queue-name-from-tenant convention is stringly-typed and duplicated**:
- [src/webhook/enqueue.rs:12](src/webhook/enqueue.rs#L12) — `format!("webhooks:{tenant_id}")`
- [src/db/stats.rs:5](src/db/stats.rs#L5) — same format, different function
- The summary query simply forgot to participate in the convention.

Dylan also wants sqlx usage that the compiler can actually check. Current state is mixed:
- `stats.rs` summary: `query_as!` (compile-checked, but logic bug)
- `stats.rs` queued count: raw `query_scalar::<Postgres, _>` (not checked)
- `dlq.rs` [list_page](src/db/dlq.rs#L47) & [count_dlq_total](src/db/dlq.rs#L30): raw strings, not checked
- `tenants.rs`, `deliveries.rs`: `query!` macros (checked)

## Recommended approach

Two moves, independent but mutually reinforcing:

### 1. Kill the stringly-typed queue name with a newtype

Introduce a tiny module — something like `src/queue.rs`:

```rust
pub struct TenantQueue(String);
impl TenantQueue {
    pub fn for_tenant(tenant_id: Uuid) -> Self { ... }
    pub fn as_str(&self) -> &str { &self.0 }
}
```

Why a newtype vs a free function? Because a `String` returned from `queue_name_for(tenant_id)` can be accidentally concatenated, logged, passed as a destination URL, etc. A newtype makes "this is a queue name bound to a tenant" a **type the compiler tracks**, and the only way to construct one is via `for_tenant`. Then every place that needs a queue name takes `&TenantQueue` or calls `.as_str()` at the sqlx boundary.

Call sites to update:
- `src/webhook/enqueue.rs:12` → build via `TenantQueue::for_tenant`
- `src/db/stats.rs:5` → same
- New usage in the fixed summary query (below)

Learning prompt for Dylan: try `Display` vs `as_str()` — which feels right when sqlx's `bind` wants `&str`? What does `impl From<Uuid> for TenantQueue` buy you vs cost you?

### 2. Split the summary into small compile-checked queries, compose concurrently

Current summary is one `query_as!` with four `COALESCE((SELECT COUNT …))` subqueries. It's fast (one round trip) but the SQL blob grows every time a new stat is added, and the jobs-count leak is exactly the kind of bug a big blob hides.

Swap it for four small `query_scalar!` (macro variant — compile-checked against `.sqlx/`) calls, composed with `tokio::try_join!`:

```rust
let (queued, delivered_today, failed_today, dlq_total) = tokio::try_join!(
    count_queued(pool, &queue),
    count_delivered_today(pool, tenant_id),
    count_failed_today(pool, tenant_id),
    count_dlq_total(pool, tenant_id),
)?;
```

Each inner fn is 3–5 lines, takes typed params (`&TenantQueue` or `&Uuid`), returns `Result<i64, sqlx::Error>`. Perf cost is negligible — four concurrent queries on one pool ≈ latency of the slowest, not the sum. And every query is individually validated against the schema at `cargo check`.

Bonus: `count_jobs_queued_today` and `count_dlq_total` already exist as the right shape — reuse them. You're really only writing two new small functions (`count_delivered_today`, `count_failed_today`) and deleting the blob.

Also migrate [src/db/dlq.rs:30](src/db/dlq.rs#L30) and [:47](src/db/dlq.rs#L47) to `query_scalar!` / `query_as!` macro forms while you're there — same reasoning, same `.sqlx/` coverage.

## Files to touch

- **new**: `src/queue.rs` (newtype) + export in `src/lib.rs`
- `src/db/stats.rs` — delete the blob, add two small `query_scalar!` fns, rewrite `webhook_activity_summary` as a `try_join!` composer, switch `count_jobs_queued_today` to `query_scalar!` macro + `TenantQueue`
- `src/webhook/enqueue.rs` — use `TenantQueue::for_tenant`
- `src/db/dlq.rs` — upgrade to macro forms (optional but consistent)
- `.sqlx/` — will need regeneration (`cargo sqlx prepare`) since new macro queries get fingerprinted

## Verification (walking skeleton: test after each small step)

1. After newtype: `cargo check` — enqueue still compiles, no behaviour change.
2. After one split query (e.g. `count_delivered_today`): call it directly from a scratch test or curl `/summary` — numbers still match pre-refactor for one tenant.
3. After full `try_join!` swap: seed two tenants with jobs, hit `/summary` with each tenant's API key, confirm `queued` differs per tenant (this is the bug repro → fix proof).
4. `cargo sqlx prepare` then `cargo check --offline` to confirm offline build still works (CI relies on it).
5. Manual: `curl -H "x-api-key: …" localhost:PORT/summary` for tenant A and tenant B — `queued` must be tenant-scoped.

## Why not alternatives (briefly)

- **Just add `queue_name = $2` to the existing blob**: fixes the bug in ~2 lines but leaves the stringly-typed drift and the opaque SQL blob that caused it. You'll hit it again on the next stat.
- **Query builder (sea-query etc.)**: overkill; sqlx macros already give compile-time checking. Adding a builder is the cargo-culted "production-looking" move the project CLAUDE.md warns against.
