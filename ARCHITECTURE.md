# HeraldQ — Architecture & Context

> Webhook delivery as a service. Accept an HTTP request, durably enqueue it,
> deliver it to a customer-specified URL with retries and signatures, and
> expose the outcome. Built in Rust on a PostgreSQL backbone.

This document is the authoritative orientation for the codebase: what each
piece does, why it's shaped that way, and where the coupling points are.

---

## 1. Product context

HeraldQ (crate name: `sliq`) is an **outbound webhook dispatcher**. A tenant
registers, receives an API key, and POSTs a payload + destination URL. HeraldQ
guarantees **at-least-once delivery** to that destination with exponential
backoff retries, HMAC-signed requests, a dead-letter queue for permanent
failures, and full delivery history.

Competitive positioning (see `webhook_business_plan.md`): a lean, pure-Rust,
low-cost alternative to Svix ($490/mo) / Hookdeck / Convoy. The pitch is
"fast, simple, and doesn't cost $490/month."

---

## 2. High-level architecture

Two binaries share one library crate (`src/lib.rs`). PostgreSQL is the only
operational dependency — it is the database, the job queue, and the durable
log of outcomes.

```
                        ┌─────────────────────────────────┐
  tenant ──HTTP──►      │            api (Axum)            │
  POST /v1/webhook      │  auth → rate-limit → SSRF check │
                        │  → quota → enqueue job           │
                        └───────────────┬─────────────────┘
                                        │ graphile_worker add_job
                                        ▼
                        ┌─────────────────────────────────┐
                        │         PostgreSQL               │
                        │  graphile_worker.jobs  (queue)   │
                        │  tenants / api_keys / quotas     │
                        │  webhook_deliveries (success)    │
                        │  dead_letter_queue   (failures)  │
                        └───────────────┬─────────────────┘
                                        │ polls job queue
                                        ▼
                        ┌─────────────────────────────────┐
                        │          worker (graphile)       │
                        │  30 concurrent SendWebhook tasks │
                        │  HTTP POST → destination URL     │
                        │  signs w/ HMAC, writes outcome   │
                        └─────────────────────────────────┘
```

### Binaries

| Binary | Entry point | Role |
|---|---|---|
| `api` | `src/bin/api.rs` | Axum HTTP server. Validates, rate-limits, enqueues. Pool size 10. |
| `worker` | `src/bin/worker.rs` | graphile_worker consumer. 30 concurrent jobs, pool size 36. Spawns the queue-depth poller and retention sweeper. |

Both binaries run migrations on startup (`db::run_migrations`) and initialise
tracing + Prometheus metrics independently — each needs its own `METRICS_PORT`.

### Shared library (`src/lib.rs`)

- `AppState` — cloned into every API handler: PgPool, `WorkerUtils` (for
  enqueue), rate limiters, the SSRF `HttpAcl` policy, schema name.
- `WorkerState` — passed as a graphile extension into every job: the
  SSRF-hardened `reqwest::Client` and PgPool.

---

## 3. The request lifecycle (the most important flow)

This is the core mental model. Trace it in the code:

```
enqueue_webhook (routes.rs:110)
  │
  ├─ auth::ValidApiKey extractor → SHA-256 hash → lookup in api_keys
  ├─ tier_rate_limits.for_tier(tier)   per-tier keyed bucket (4/20/50 req/s)
  ├─ ssrf::validate_destination(policy, url)   HTTPS-only, no private IPs
  ├─ db::quotas::increment_monthly_quota()   atomic, tier-sized cap (5k–5M/mo)
  └─ webhook::enqueue() → graphile add_job(SendWebhook, job_key=ULID)
        │
        ▼  (job persists in graphile_worker.jobs)

SendWebhook::run (webhook/dispatch.rs:36)   ← worker picks it up
  │
  ├─ look up tenant's hmac_secret, sign the body (t=timestamp.v1=sig)
  ├─ HTTP POST with Idempotency-Key + HeraldQ-Signature headers
  │
  └─ decision table on response status:
       2xx         → Success  → db::deliveries::insert (idempotent ON CONFLICT)
       408/425/429  → Retry
       5xx         → Retry
       timeout/conn→ Retry (unless SSRF block → PermaFail)
       other 4xx   → PermaFail → db::dlq::insert
       3xx         → PermaFail (redirects NOT followed)

  Retry exhaustion (attempts >= max_attempts) → db::dlq::insert
```

**Three terminal states, one status endpoint.** `GET /v1/webhook/{id}`
(`webhook_status`, routes.rs:423) checks, in order: deliveries (success),
DLQ (failure), then the live job queue (pending). It re-checks terminal
stores once more before 404 to close the race where a job completes between
checks. The `status` JSON tag (`delivered` / `failed` / `pending`) is a
**public API contract** — pinned by a test.

### Retry schedule

Set in `enqueue.rs`: `max_attempts(15)` with graphile's exponential backoff
(`e^n` capped at `e^10` ≈ 6.1h). Spans ~34 hours total:
2.7s, 7.4s, 20s, 55s, 2.5m, 6.7m, 18m, 50m, 2.25h, then 6h between each.
Documented for customers in `docs/delivery-behavior.md` — change them together.

---

## 4. Database schema

SQLx migrations in `migrations/` (15 so far), checked into `.sqlx/` for
offline compile-time validation (`SQLX_OFFLINE=true`).

### Application tables

| Table | Purpose | Key columns |
|---|---|---|
| `tenants` | Account root | `id` (UUID), `tier` (enum), `email` (unique), `hmac_secret`, retention days |
| `api_keys` | Hashed credentials | `key_hash` (SHA-256, unique), `expires_at`, `is_active` |
| `tenant_monthly_quotas` | Usage cap | `(tenant_id, month_start)` PK, `webhook_count` |
| `webhook_deliveries` | Success log | `id` (ULID), `payload` (JSONB), `delivered_at`, `attempts` |
| `dead_letter_queue` | Failure log + replay | `id`, `failure_reason`, `replayed_at`, `replayed_as` |

### graphile_worker tables (managed by the library)

The job queue lives in the `GRAPHILE_SCHEMA` schema (default `graphile`).
Two internals we read directly, coupled to graphile's private schema:

- `graphile_worker._private_jobs` — full payload, read by
  `db::jobs::find_by_webhook_id` and `db::stats::count_jobs_queued_today`.
  The public `jobs` view omits `payload`.
- `graphile_worker.jobs` — read by the queue-depth poller for backpressure
  gauges (`heraldq_queue_depth`, `heraldq_queue_oldest_job_age_seconds`).

> **Upgrade hazard:** any `graphile_worker` version bump requires
> re-verifying these queries against the private schema. Search for
> `_private_jobs` and the caveats in `db::jobs.rs` / `db::stats.rs`.

### Indexes (added incrementally as needed)

- `idx_dlq_tenant_id` — DLQ tenant scoping
- `idx_deliveries_tenant_delivered` — the dashboard's hot query
- `idx_deliveries_delivered_at` / `idx_dlq_abandoned_at` — retention sweeper
- graphile's own unique index on `jobs.key` — powers status lookups by ULID

---

## 5. Security model

### Authentication

Bearer-token API keys (`auth/mod.rs`). The raw key is never stored: it is
SHA-256 hashed on receipt and looked up in `api_keys`. Keys are prefixed
`sliq_` + 32 hex chars (16 random bytes). Keys can expire (`expires_at`,
migration 0004). The `ValidApiKey` extractor is an Axum `FromRequestParts`
applied to protected routes — missing/invalid/expired → 401.

### HMAC webhook signing

Every outbound delivery is signed (`webhook/dispatch.rs:sign_bytes`):

```
signed_content = "{timestamp}.{raw_body}"
signature = hex(HMAC_SHA256(tenant_hmac_secret, signed_content))
header: HeraldQ-Signature: t=<unix_ts>,v1=<hex_sig>
```

The secret is per-tenant (`tenants.hmac_secret`), generated at signup, and
returned **once**. Verification docs in `docs/webhook-signatures.md`. Each
retry attempt is signed fresh (new timestamp) so the receiver's replay
window doesn't reject legitimate retries.

### SSRF protection (defense in depth)

The destination URL is validated at **two** layers:

1. **Enqueue time** (`ssrf::validate_destination`): parse URL, require HTTPS,
   reject embedded credentials, screen literal IPs against the `HttpAcl`.
2. **Connect time** (`ssrf::SecureResolver`): a custom `reqwest::dns::Resolve`
   that screens every resolved address. A split-DNS answer mixing public +
   private IPs is rejected wholesale — this defeats DNS rebinding.

Policy (`ssrf::policy`): allow any global IP, deny the entire non-global set
(loopback, RFC1918, link-local + cloud metadata `169.254.169.254`, CGNAT,
IPv6 ULA). The worker's HTTP client also disables redirects and proxy
following. Tests pin the classic evasion vectors (decimal/hex/octal IPs).

### Rate limiting & quotas

| Control | Mechanism | Limit |
|---|---|---|
| Enqueue rate | `governor` keyed buckets per tier (`TierRateLimits`) | 240 / 1.2k / 3k req per minute by tier |
| Global signup rate | `governor` direct limiter | 100/min across all clients |
| Monthly volume | Atomic DB upsert, sized by `tiers::monthly_webhook_limit` | 5k / 50k / 500k / 5M by tier |
| API key validity | `expires_at` column + check | No expiry; tier/quota is the boundary |

The monthly quota is race-free: a single `INSERT ... ON CONFLICT DO UPDATE
WHERE count < limit RETURNING` either increments and returns a row, or
returns nothing (quota exceeded). No read-then-write gap.

---

## 6. The dead-letter queue & replay

When a webhook permanently fails (4xx, redirects) or exhausts 15 retries, it
lands in `dead_letter_queue` with the failure reason and last status code.

**Self-serve replay** (`dlq_replay`, routes.rs:361) re-enqueues a DLQ entry as
a fresh delivery. The concurrency design is worth understanding:

1. Mint a new ULID up front.
2. `claim_for_replay` — atomic `UPDATE ... WHERE replayed_at IS NULL
   RETURNING`. First caller wins; a double-click gets 409 with the existing
   replay id.
3. Re-validate SSRF (policy may have changed since original enqueue).
4. Enqueue under the new id. On any failure, `unclaim_replay` releases the
   row so the tenant can retry.

The original DLQ row is **kept** (stamped `replayed_at`/`replayed_as`) — it's
still the record of the original failure.

---

## 7. Idempotency & crash safety

The system is designed for graphile's **at-least-once** job execution. If the
worker crashes after delivering a webhook but before marking the job complete,
the job re-runs with the same ULID. Both terminal-state inserts use
`ON CONFLICT (id) DO NOTHING` so a re-run converges on the already-recorded
outcome instead of erroring into a poison loop. See `db::deliveries.rs` and
`db::dlq.rs`.

The `SendWebhook` struct (`webhook/model.rs`) is a **persisted wire format** —
rows written by one deploy are deserialized by the next. Wire-format tests pin
backwards/forwards compatibility. Never add `deny_unknown_fields`.

---

## 8. Observability

### Metrics (`telemetry.rs`)

Both binaries export Prometheus metrics on `/metrics` when `METRICS_PORT` is
set. **Unset, every `metrics::` macro is a no-op** — a process with no
`METRICS_PORT` is invisible. The exporter binds `::` (dual-stack) by default
for Railway's IPv6-only private network.

Key metrics ("process" matters — each binary exports only what it runs):

| Metric | Process | Source | Type |
|---|---|---|---|
| `heraldq_http_requests_total{method,path,status}` | api | middleware | counter |
| `heraldq_http_request_duration_seconds{method,path}` | api | middleware | histogram |
| `heraldq_webhooks_enqueued_total{outcome}` | api | enqueue handler | counter |
| `heraldq_signups_total{outcome}` | api | signup handler | counter |
| `heraldq_webhook_delivery_duration_seconds` | worker | dispatch.rs | histogram |
| `heraldq_webhooks_delivered_total` | worker | dispatch.rs | counter |
| `heraldq_webhooks_attempts_retried_total` | worker | dispatch.rs | counter |
| `heraldq_webhooks_dead_lettered_total` | worker | dispatch.rs | counter |
| `heraldq_dlq_write_failures_total` | worker | dispatch.rs | counter (critical) |
| `heraldq_retention_deleted_total{table}` | worker | retention sweeper | counter |
| `heraldq_queue_depth` / `heraldq_queue_oldest_job_age_seconds` | worker | poller | gauge |

Naming has one historical quirk: delivery metrics are singular
(`webhook_delivery_duration`) while outcome counters are plural
(`webhooks_delivered_total`). Kept as-is — renaming churns dashboards and
alert rules for zero function. New metrics: outcome counters are plural
(`..._total{outcome=...}`), everything else singular.

Alerts live in exactly one place, `alerts/heraldq.rules.yml` — evaluated
by local Prometheus (`observability/prometheus.sh`) and by Grafana
Cloud's ruler in production. Dashboards in
`observability/grafana/dashboards/heraldq.json` (Prometheus for rates
and gauges, Postgres datasource for per-tenant forensics).

**Cardinality discipline:** the `path` label is the route template
(`/v1/webhook/{id}`), never the concrete URI — `MatchedPath` prevents
per-ULID series. Method is collapsed to a closed `&'static str` set. Per-
endpoint failure leaderboards live in SQL (Postgres datasource), not in
Prometheus labels.

Histograms get explicit buckets (not default summaries) so they remain
aggregatable across replicas for fleet-wide `histogram_quantile()`.

### Alerting (`alerts/heraldq.rules.yml`)

Six rules, load by both local Prometheus and Grafana Cloud's ruler:

- **HeraldQDlqWriteFailure** (critical) — any DLQ write failure in 5m. The one
  silent-data-loss path in the system.
- **HeraldQDeadLetterSpike** (warning) — sustained dead-letter rate > 0.05/s.
- **HeraldQQueueLagging / Stalled** (warning/critical) — head-of-line job age
  > 60s / 600s. Age, not depth, is the backpressure signal.
- **HeraldQApiErrorRate** (critical) — 5xx ratio > 5% (4xx excluded).
- **HeraldQApiLatencyHigh** (warning) — p99 > 1s for 10m.
- **HeraldQTargetDown** (critical) — a scrape target unreachable for 2m.

### Dashboards & local stack

Grafana dashboard at `observability/grafana/dashboards/heraldq.json`, viewable
at `/d/heraldq-overview`. Queries **both** Prometheus and Postgres (two
datasources by design — see README "Why two datasources"). Local stack:
`observability/prometheus.sh` + `observability/grafana.sh`.

### Tracing

`tracing` + `tracing-subscriber` with `EnvFilter`. `LOG_FORMAT=json` for log
aggregators (flattened fields). Every request gets an `x-request-id` via
tower-http, visible in the trace span and propagated to the response. Log
level convention: `info!` milestones, `debug!` payloads/state, `trace!` noise.

---

## 9. Error handling (`error.rs`)

`AppError` is the single error enum for the API layer, implementing
`IntoResponse`. Key principle: **internal details are logged but never leak
into response bodies.** `sqlx::Error` and `GraphileWorkerError` map to a
generic 500 with body "Internal server error" (tested explicitly). Domain
errors (`Unauthorized`, `NotFound`, `AlreadyReplayed`,
`InvalidDestination`) map to precise status codes.

---

## 10. Retention (`db/retention.rs`)

A background sweeper (worker binary only, hourly) deletes terminal rows past
each tenant's retention window (`delivered_retention_days` default 30,
`dlq_retention_days` default 90). Deletes run in batches of 1,000 to keep
lock footprint and WAL modest. Per-tenant retention is read at execution
time, so changing a tenant's window takes effect on the next sweep.

---

## 11. Configuration

Environment-driven (no config files). Required:

| Var | Required | Default | Notes |
|---|---|---|---|
| `DATABASE_URL` | yes | — | Postgres connection string |
| `GRAPHILE_SCHEMA` | yes | — | Schema for graphile_worker tables |
| `PORT` | no | `3000` | API listen port |
| `METRICS_PORT` | no | — | Prometheus exporter; unset = no-op |
| `METRICS_BIND` | no | `::` | Override bind address |
| `LOG_FORMAT` | no | human | `json` for log aggregators |
| `RUST_LOG` | no | — | tracing filter (e.g. `sliq=debug`) |
| `SERVICE_NAME` | no | `api` | Dockerfile multi-service selector |
| `PADDLE_WEBHOOK_SECRET` | no | — | Paddle notification-destination secret; unset = `/webhooks/paddle` refuses deliveries (503) |

Loaded via `AppConfig::from_env()` (`config.rs`), which calls `dotenvy::dotenv()`
first for local `.env` files.

---

## 12. Deployment

- **Platform:** Railway (`railway.toml` defines two services: `api` + `worker`).
- **Containers:** `cargo-chef` multi-stage builds for dependency caching.
  `Dockerfile.api` and `Dockerfile.worker` are the service-specific images
  (used by Railway). A combined `Dockerfile` exists with `SERVICE_NAME` selector.
- **Build:** `SQLX_OFFLINE=true` in Docker + CI — queries validated against
  committed `.sqlx/` metadata, no database needed. Regenerate with
  `cargo sqlx prepare` after query changes.
- **Graceful shutdown:** API listens for SIGINT *and* SIGTERM (Docker/K8s/
  systemd use SIGTERM) and drains in-flight requests.

---

## 13. Testing strategy

- **Unit tests** inline in modules: config parsing, auth extraction, SSRF
  policy, error→status mapping, wire-format compatibility, status response
  serialization. Run in CI (`cargo test`) with `SQLX_OFFLINE=true`.
- **Integration tests** (`tests/`): DLQ replay claim semantics, retention
  sweeps, API metrics, terminal-write idempotency. These need a live Postgres
  (`#[sqlx::test]`) and are `#[ignore]`d in CI — run locally with
  `cargo test -- --ignored`.
- **CI** (`.github/workflows/ci.yml`): `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`. No database.

---

## 14. API surface (v1)

All under `/v1`, protected by Bearer auth unless noted.

| Method | Route | Auth | Description |
|---|---|---|---|
| GET | `/ping` | no | Liveness (`pong`) |
| GET | `/health` | no | Health JSON |
| POST | `/webhooks/paddle` | HMAC signature | Paddle billing webhooks → dedup + mirror subscription state |
| POST | `/v1/signup` | no (rate-limited) | Open tenant registration → returns API key + HMAC secret; links any existing Paddle customer and applies its tier |
| GET | `/v1/keys` | yes | Key metadata (never hashes or raw values) |
| POST | `/v1/keys/rotate` | yes | Revoke all keys, issue one replacement |
| POST | `/v1/keys/recover` | no (rate-limited) | Lost-key recovery via email + HMAC secret |
| POST | `/v1/webhook` | yes | Enqueue a webhook → `202 { id }` |
| GET | `/v1/webhook/{id}` | yes | Status (delivered / failed / pending) |
| GET | `/v1/deliveries` | yes | Paginated success history |
| GET | `/v1/dlq` | yes | Paginated dead-letter queue |
| POST | `/v1/dlq/{id}/replay` | yes | Re-enqueue a DLQ entry → `202 { id, replayed_from }` |
| GET | `/v1/summary` | yes | JSON activity summary |

---

## 15. Codebase map

```
src/
├── lib.rs              AppState, WorkerState, module root
├── config.rs           AppConfig (env parsing) + tests
├── error.rs            AppError → HTTP status mapping + tests
├── routes/
│   ├── mod.rs          router composition, middleware stack, CORS scoping,
│   │                   /ping + /health, shared pagination
│   ├── accounts.rs     POST /v1/signup + email validation
│   ├── keys.rs         GET /v1/keys, POST /v1/keys/rotate, /v1/keys/recover
│   ├── webhooks.rs     POST /v1/webhook, GET /v1/webhook/{id}
│   ├── dlq.rs          GET /v1/dlq, POST /v1/dlq/{id}/replay
│   ├── stats.rs        GET /v1/deliveries, /v1/summary
│   └── billing.rs      POST /webhooks/paddle
├── telemetry.rs        tracing init, Prometheus exporter, metrics middleware,
│                       queue-depth poller
├── bin/
│   ├── api.rs          API server entry point
│   └── worker.rs       Worker entry point
├── auth/
│   └── mod.rs          API key gen/hash/verify, ValidApiKey extractor
├── paddle/
│   ├── mod.rs          event dispatch + delivery-contract docs
│   ├── signature.rs    Paddle-Signature HMAC verification + tests
│   ├── events.rs       serde models for webhook payloads
│   └── sync.rs         customer/subscription mirrors, dedup ledger, tier propagation
├── webhook/
│   ├── mod.rs          module root + re-exports
│   ├── model.rs        WebhookRequest, SendWebhook (job payload) + wire tests
│   ├── enqueue.rs      graphile add_job, retry config
│   ├── dispatch.rs     TaskHandler: sign, POST, outcome decision table
│   └── ssrf.rs         validate_destination, SecureResolver, policy + tests
└── db/
    ├── mod.rs          pool creation, migrations
    ├── tenants.rs      create_tenant, api_keys, hmac secret, recovery lookup
    ├── quotas.rs       atomic monthly quota increment
    ├── jobs.rs         pending-job lookup via graphile private schema
    ├── deliveries.rs   success log CRUD (idempotent insert)
    ├── dlq.rs          DLQ CRUD, claim/unclaim replay
    ├── retention.rs    batched sweep sweeper
    └── stats.rs        activity summary counts
```

---

## 16. Conventions & gotchas

- **Handlers live in `routes/`, one module per resource.** `routes/mod.rs`
  is composition only — if you are adding a handler body there, it belongs
  in a submodule instead. Handlers are `pub(super)`: the router is the only
  way in.
- **CORS scoping is positional.** `route_layer` wraps only the routes
  registered *before* it, so the two browser-facing routes (`/v1/signup`,
  `/v1/keys/recover`) are declared in `build()` rather than in their own
  modules. Adding a browser-callable route means adding it there, not just
  anywhere.
- **Names are split:** the crate/binary is `sliq`, the product is HeraldQ,
  the header is `HeraldQ-Signature`, the metrics prefix is `heraldq_`.
- **Migration 0005 `TRUNCATE`s tenants** — it was a one-time schema reset for
  adding `email`/`hmac_secret`. Existing migrations are append-only history;
  never edit them.
- **`max_attempts` appears in two places:** the `enqueue.rs` job spec (15)
  and the README (3, outdated). The authoritative value is 15; the delivery-
  behavior doc documents the full ~34h schedule.
- **graphile private-schema coupling:** searches for `_private_jobs` and
  comments mentioning "re-verify on upgrades" mark every spot that breaks
  if graphile_worker changes its internals.
- **`SendWebhook` is a wire format:** see model.rs doc comment. Adding fields
  requires `#[serde(default)]`; never add `deny_unknown_fields`.
- **The `[TRACER BULLET]` log line** in dispatch.rs DLQ writes is a leftover
  from initial implementation — cosmetic, not functional.
