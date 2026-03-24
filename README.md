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
| `POST /mock` | Chaos endpoint — returns 200 (10%), 418 (5%), 500 (85%) |

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
cargo run --bin api     # starts on PORT (default 3000)
cargo run --bin worker
```

Both binaries are also containerised — see `Dockerfile.api` and `Dockerfile.worker`.

To sign up and receive an API key

```bash
curl -X POST your-service.com/signup \
  -H 'Content-Type: application/json' \
  -d '{"name": "My Company"}'
```

Example payload

```bash
curl -w "%{http_code}\n" 
    -X POST http://localhost:3000/webhook     
    -H "Authorization: Bearer $SLIQ_KEY"     
    -H "Content-Type: application/json"     
    -d '{ "payload": {"hello": "dyl" }, "destination": "http://localhost:3000/mock" }'
```

## Tracing 
Add `debug!` and `trace!` calls at the points you care about — payload contents, retry decisions, response bodies. These are your "`printf` but permanent." The convention:
- `info!` — happy-path milestones (what you'd want in prod logs)
- `debug!` — the stuff you're asking about now: payloads, intermediate state
- `trace!` — really noisy stuff like raw HTTP bodies, loop iterations

Run with the right filter — `RUST_LOG=sliq_graphile::webhook=debug` and you see exactly that module's debug output without noise from hyper/tokio/sqlx.

Structured fields are your friend — you're already doing 
```rust
info!(job_id = %job_id, ...)
```
For payloads, same pattern: 
```rust
debug!(payload = ?payload, url = %url, "dispatching webhook")
``` 
The `?` uses Debug formatting, `%` uses Display.