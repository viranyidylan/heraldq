# MVP Build Checklist

## Week 1 — Core Correctness + Auth

### Session 1 (2–3 hrs) — Fix critical bugs
- [ x ] Fix `try_dispatch_webhook` to handle 4xx (permanent fail) and 5xx (retry) correctly
- [ x ] Wire up `permanently_fail_jobs` via `WorkerContext` for 4xx
- [ x ] Fix `Ok(())` swallowing non-2xx responses
- [ x ] Replace `"Hi Dylan"` root with `GET /health` returning `{ "status": "ok" }`
- [ x ] Wire up `shutdown_signal()` to Axum `with_graceful_shutdown()`
- [ x ] Use shared `SCHEMA` constant in both binaries

### Session 1.2 (1 hr) — Test all bugfixes
- [ x ] Fix `try_dispatch_webhook` to handle 4xx (permanent fail) and 5xx (retry) correctly
- [ x ] Wire up `permanently_fail_jobs` via `WorkerContext` for 4xx
- [ x ] Fix `Ok(())` swallowing non-2xx responses
- [ x ] Replace `"Hi Dylan"` root with `GET /health` returning `{ "status": "ok" }`
- [ x ] Wire up `shutdown_signal()` to Axum `with_graceful_shutdown()`
- [ x ] Use shared `SCHEMA` constant in both binaries

### Session 2 (2-3 hrs) - Include notion of a "tenant" along with their api keys
- [ x ] Add `tenant` table
- [ x ] Add `api_keys` table
- [ x ] Write migration SQL for `api_keys` table
- [ x ] Write migration SQL for `tenant` table
- [ x ] Wire `sqlx::migrate!` macro into API startup

### Session 4 (2–3 hrs) — Auth middleware
- [ ] SHA-256 hash incoming bearer token
- [ ] Axum extractor that looks up hash in `api_keys`
- [ ] 401 on missing/invalid/inactive key
- [ ] Apply extractor to `POST /webhook` only

### Session 5 (2-3 hrs) - Dead Letter Queue
- [ ] Write migration SQL for `webhook_dlq` table
- [ ] Write to SQL each place we perma-fail 
- [ ] Update any migration we need

### Session 8 (1–2 hrs) — Rate limiting
- [ ] Wire `tower_governor` to `POST /webhook`
- [ ] Per-key rate limits

---

## Week 2 — DLQ + Observability + Status

### Session 4 (2–3 hrs) — Dead Letter Queue
- [ ] Fill in `handle_result()` stub in `lib.rs`
- [ ] Write to `webhook_dlq` on permanent failure (4xx or max attempts exceeded)
- [ ] `GET /dlq` route — paginated, newest-first, tenant-scoped, auth protected

### Session 6 (2–3 hrs) — Status endpoint + UI
- [ ] `GET /status` JSON: `{ queued_today, dispatched_today, failed_today, dlq_total, uptime_seconds }`
- [ ] Drive numbers from COUNT queries against graphile tables + `webhook_dlq`
- [ ] `GET /status.html` inline HTML page, no template engine, vanilla fetch()

### Session 7 (1–2 hrs) — Structured logging
- [ ] Add `tracing` + `tracing-subscriber` with JSON formatter + `EnvFilter`
- [ ] Log: job enqueued, dispatch attempt #, success, failure with error string
- [ ] Initialize in both `main()` functions

### Session 8 (1–2 hrs) — Rate limiting
- [ ] Wire `tower_governor` to `POST /webhook`
- [ ] 100 req/min per IP as starting point

### Session 9 (1–2 hrs) — Some decent basic metrics graph/dashboard
- [ ] start with plan mode to think through the absolute basics for some metrics dashboard


---

## Week 2.5 — Deploy + Smoke Test + Stress test + Ship

### Session 9 (2–3 hrs) — Railway deploy
- [ ] Verify `railway.toml` service split (api + worker) builds cleanly
- [ ] Confirm both Dockerfiles build for `--release`
- [ ] Set `DATABASE_URL` on both Railway services
- [ ] Confirm migrations run on startup
- [ ] Smoke test: enqueue via curl → verify delivery → check `/status`

### Session 10 (2 hrs) — End-to-end hardening
- [ ] Test retry path: 500 endpoint → retries on schedule → lands in DLQ after 6 attempts
- [ ] Test permanent fail: 4xx endpoint → immediate DLQ, no retries
- [ ] Test auth: 401 on missing/wrong key
- [ ] Seed one API key per early test customer
- [ ] Share `/status.html` URL with first users

---

## Deferred backlog (Post-Launch)
- HMAC signature verification on outbound webhooks
- Per-key rate limits
- Requeue from DLQ
- Full 30-day delivery history
- Dashboard UI framework
- Per-tenant usage metrics
- DLQ cleanup job? (90 day retention policy)
- Split out the lib.rs into some more specific modules
- Upgrade tenant IDs to uuid v7
- Basic web page