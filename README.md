# test_graphile

A Rust webhook dispatch service backed by [graphile_worker](https://docs.rs/graphile_worker/latest/graphile_worker) — a PostgreSQL-based job queue.

## Architecture

Two binaries share a single library crate (`src/lib.rs`):

```
POST /webhook  →  [api]  →  PostgreSQL job queue  →  [worker]  →  target URL
```

### `api` (Axum HTTP server)
Accepts incoming webhook requests and enqueues them as durable jobs via `WorkerUtils`. Jobs persist in PostgreSQL, so they survive restarts.

| Route | Description |
|---|---|
| `GET /ping` | Liveness check |
| `GET /health` | Health check (JSON) |
| `POST /webhook` | Enqueue a webhook dispatch job |

### `worker` (graphile_worker consumer)
Polls the job queue and dispatches `POST` requests to the destination URL.

- **Concurrency:** 30 parallel jobs
- **Local queue:** batches up to 100 jobs from DB to reduce round-trips; refetches when queue drops below 10
- **Batch completions:** job completions are flushed every 5ms to reduce SQL round-trips

### Shared (`lib.rs`)
Defines the `SendWebhook` task and its retry/failure logic:

- **Retry:** server errors (5xx) are retried up to `max_attempts` (default: 3)
- **Permanent failure:** client errors (4xx) are immediately permanently failed via `permanently_fail_jobs`
- **DLQ:** once `max_attempts` is exhausted, the failure is logged (TODO: write to a DLQ table in PostgreSQL)
- **Idempotency:** each job carries a ULID passed as an `Idempotency-Key` header to the target

## Stack

| | |
|---|---|
| Language | Rust (edition 2024) |
| HTTP server | `axum` |
| Job queue | `graphile_worker` (PostgreSQL-backed) |
| Database | PostgreSQL via `sqlx` |
| IDs | `ulid` |

## Running locally

You may need to run scripts/setup_db.sh to initialize the DB and user

Set `DATABASE_URL` (defaults to `postgres://postgres:xi23pe@localhost/sliq`), then:

```bash
METRICS_PORT=9464 cargo run --bin api     # starts on PORT (default 3000)
METRICS_PORT=9465 cargo run --bin worker
```

Both binaries are also containerised — see `Dockerfile.api` and `Dockerfile.worker`.

To sign up and receive an API key

```bash
curl -X POST your-service.com/v1/signup \
  -H 'Content-Type: application/json' \
  -d '{"name": "My Company", "email": "you@example.com" }'
```

Example payload

```bash
curl -w "%{http_code}\n" \
    -X POST http://localhost:3000/v1/webhook \
    -H "Authorization: Bearer $SLIQ_KEY" \
    -H "Content-Type: application/json" \
    -d '{ "payload": {"hello": "dyl" }, "destination": "https://httpbin.org/status/200" }'
```

Note the destination must be publicly routable — the SSRF guard rejects
localhost and private IPs, so `http://localhost:3000/anything` is refused
with a 400. That is the guard working, not a bug.

## Observability

Both binaries export Prometheus metrics on `/metrics` when `METRICS_PORT` is
set. **Unset, every `metrics::` macro silently becomes a no-op** — so a process
with no `METRICS_PORT` is invisible, not merely quiet.

The full local stack is four processes:

```bash
METRICS_PORT=9464 cargo run --bin api      # :3000, metrics on :9464
METRICS_PORT=9465 cargo run --bin worker   #        metrics on :9465
bash observability/prometheus.sh           # :9090 — scrapes both, evaluates alerts/
bash observability/grafana.sh              # :3001 — the dashboard
```

Then open **http://localhost:3001/d/heraldq-overview**.

- `http://localhost:9090/targets` — both jobs should be UP. A DOWN target means
  that process is stopped or has no `METRICS_PORT`.
- `http://localhost:9090/alerts` — the rules in `alerts/heraldq.rules.yml`,
  live. This is the same file Grafana Cloud's ruler evaluates in production, so
  a rule proven here is proven in prod.

### Why two datasources

The dashboard queries Prometheus **and** Postgres, on purpose. They answer
different classes of question and neither can do the other's job:

| | Prometheus | Postgres |
|---|---|---|
| Answers | rates, percentiles, queue depth over time | which endpoint failed, what's in the DLQ, per-tenant history |
| Cardinality | bounded — labels are a closed set | unbounded by design |
| Used for | alerting, "is it healthy right now" | forensics, "what exactly happened" |

A per-endpoint failure leaderboard as a Prometheus label would be one time
series per customer URL — a cardinality bomb. It lives in SQL. Conversely, a
fleet-wide p99 cannot be computed from SQL rows at scrape speed. Route each
question to the store that can answer it.

## Tracing 
Add `debug!` and `trace!` calls at the points you care about — payload contents, retry decisions, response bodies. These are your "`printf` but permanent." The convention:
- `info!` — happy-path milestones (what you'd want in prod logs)
- `debug!` — the stuff you're asking about now: payloads, intermediate state
- `trace!` — really noisy stuff like raw HTTP bodies, loop iterations

Run with the right filter — `RUST_LOG=sliq=trace,api=trace,worker=debug` and you see exactly that module's debug output without noise from hyper/tokio/sqlx.

Structured fields are your friend — you're already doing 
```rust
info!(job_id = %job_id, ...)
```
For payloads, same pattern: 
```rust
debug!(payload = ?payload, url = %url, "dispatching webhook")
``` 
The `?` uses Debug formatting, `%` uses Display.