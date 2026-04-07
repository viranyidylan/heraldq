# MVP Build Checklist
## Done
- [x] Fix `try_dispatch_webhook` to handle 4xx (permanent fail) and 5xx (retry) correctly
- [x] Wire up `permanently_fail_jobs` via `WorkerContext` for 4xx
- [x] Fix `Ok(())` swallowing non-2xx responses
- [x] Replace `"Hi Dylan"` root with `GET /health` returning `{ "status": "ok" }`
- [x] Wire up `shutdown_signal()` to Axum `with_graceful_shutdown()`
- [x] Use shared `SCHEMA` constant in both binaries
- [x] Add `tenant` + `api_keys` tables with migration SQL
- [x] Wire `sqlx::migrate!` macro into API startup
- [x] SHA-256 hash incoming bearer token
- [x] Axum extractor that looks up hash in `api_keys`
- [x] 401 on missing/invalid/inactive key
- [x] Apply extractor to `POST /webhook` only
---
## Project Structure Refactor
- [x] config.rs
- [x] error.rs
- [x] auth/mod.rs
- [x] webhook/model.rs + webhook/dispatch.rs
- [x] db/mod.rs + db/dlq.rs
---
### Dead Letter Queue
- [x] Write migration SQL for `webhook_dlq` table
- [x] Write to DLQ each place we permanent-fail
- [x] `GET /dlq` — paginated, newest-first, tenant-scoped, auth protected
--
## Go-to-Market Minimum
- [x] `GET /status` JSON: `{ queued_today, dispatched_today, failed_today, dlq_total, uptime_seconds }`
- [x] Drive numbers from COUNT queries against graphile tables + `webhook_dlq`
- [x] `GET /dlq` — paginated, newest-first, tenant-scoped, auth protected
- [x] Self-serve API key provisioning
- [x] `GET /summary.html` — inline HTML page, vanilla fetch()
- [] HMAC-SHA256 payload signing on outbound webhooks — table stakes, every serious buyer asks
- [] Stripe integration + single-page pricing ($29 / $79 / $199)
- [] One-curl demo that proves value end-to-end: `curl -X POST .../api/v1/webhooks` → delivers to a test URL → visible in `/status`

## Up Next — Type System + Security Hardening

### SSRF hardening — ValidatedUrl newtype (parse, don't validate)
- [ ] Add `url` crate to Cargo.toml
- [ ] Create `src/webhook/destination.rs` with `ValidatedUrl` newtype (private inner field)
- [ ] `ValidatedUrl::parse()`: enforce scheme (https only, http+https in dev), reject private IPs, loopback, link-local 169.254.0.0/16, metadata hostnames
- [ ] Implement `Deserialize` for `ValidatedUrl` — validation fires at deserialization, invalid destinations never reach handlers
- [ ] Change `WebhookRequest.destination` and `SendWebhook.destination` to `ValidatedUrl`
- [ ] Create `DestinationError` enum, wire into `AppError` as 400 (not 500)
- [ ] Tests: private IPs rejected, loopback rejected, metadata endpoint rejected, valid HTTPS passes, bare IPs rejected

### Strong ID types — stop storing typed IDs as String
- [ ] `SendWebhook.id` → `ulid::Ulid` (not `String`)
- [ ] `ValidApiKey.api_key_id` → `Uuid` (not `String`)
- [ ] Follow the compiler errors — every serialization/deserialization boundary that touches these fields needs updating
- [ ] Wire `Ulid` serde support through job queue payload

### SendWebhook type split — separate domain/job/dispatch concerns
- [ ] Extract a dispatch-only struct (e.g. `DispatchRequest { destination, payload, idempotency_key }`) for `dispatch_http`
- [ ] Keep `SendWebhook` as the job queue payload type with `TaskHandler` impl
- [ ] `run()` constructs the dispatch struct from `SendWebhook` fields
- [ ] DB persistence functions take only the fields they need

---

### Structured Logging
- [ x ] Add `tracing` + `tracing-subscriber` with JSON formatter + `EnvFilter`
- [ x ] Log: job enqueued, dispatch attempt, success, failure with error context
- [ x ] Initialize in both `main()` functions

### Rate Limiting
- [ ] Wire `tower_governor` to `POST /webhook`
- [ ] 1000 req/min per IP as starting point
- [ ] Per-key rate limits

### Status + Observability
- [ x ] `GET /status` JSON: `{ queued_today, dispatched_today, failed_today, dlq_total, uptime_seconds }`
- [ x ] Drive numbers from COUNT queries against graphile tables + `webhook_dlq`
- [ ] `GET /status.html` — inline HTML page, vanilla fetch()
- [ ] Basic metrics dashboard (plan mode first to scope it)

---

### Railway Deploy
- [ ] Verify `railway.toml` service split (api + worker) builds cleanly
- [ ] Confirm both Dockerfiles build for `--release`
- [ ] Set `DATABASE_URL` on both Railway services
- [ ] Confirm migrations run on startup
- [ ] Smoke test: enqueue via curl → verify delivery → check `/status`

### End-to-End Hardening
- [ ] Retry path: 500 endpoint → retries on schedule → lands in DLQ after max attempts
- [ ] Permanent fail: 4xx endpoint → immediate DLQ, no retries
- [ ] Auth: 401 on missing/wrong key
- [ ] Seed one API key per early test customer
- [ ] Share `/status.html` URL with first users

---

## Deferred Backlog (Post-Launch)
- HMAC signature verification on outbound webhooks
- Requeue from DLQ
- Full 30-day delivery history
- Dashboard UI framework
- Per-tenant usage metrics
- DLQ cleanup job (90-day retention policy)
- Upgrade tenant IDs to UUID v7
- Basic landing page
