
- [x] per tenant hmac secret and signup_codes table
- [x] hmac secret generation logic
- [x] update /signup to require email and invite_code
- [x] dispatch.rs loads the per-tenant hmac secret
- [x] create a script that seeds some invite codes

then 
- [x] railway deploy and smoke-test:
 - [x] Confirm both Dockerfiles build --release locally.
 - [x] Push; set DATABASE_URL on api + worker services.
 - [x] Watch logs: migrations ran, both services green.
 - [x] Smoke sequence via the deployed URL:
    - 1. POST /signup with a beta code → grab api_key + hmac_secret
    - 2. POST /webhook with bearer api_key, destination = webhook.site URL → 202
    - 3. Check webhook.site received it with valid Sliq-Signature
    - 4. GET /summary → delivered count = 1
- [x] Verify: all four above pass against the live Railway URL.
- [x] Onboarding / invite / payment flow:
    - [x] Invite code
    - [x] Trial key
    - [x] HMAC generation and storage
- [] One-curl demo that proves value end-to-end: `curl -X POST .../api/v1/webhooks` → delivers to a test URL → visible in `/status`

## Address before taking 1st customer:
- [] Make "Connection refused" a retryable failure
- [] Make rate limiter on POST /webhooks per-tenant
- [] SSRF hardening (validate urls, require https, resolve the host ourselves)
- [] Encrypt HMAC secrets or store them in a secrets manager
- [] Enforce a default body size limit for webhook requests

## Blocking demo
- [x] Migration: per-tenant hmac_secret + beta_codes table
- [x] Signup requires invite code, returns api_key + hmac_secret once
- [x] Dispatch reads hmac_secret per-tenant (drop env var)
- [x] Seed ~10 beta codes
- [x] Railway deploy + end-to-end smoke test (signup → webhook → summary)
- [] Record 90s Loom curl demo
- [x] Get your dylan@heraldq.com email setup
- [x] get your dylan@heraldq.com twitter/X setup
- [] Indie Hackers post + 10 direct outreach DMs

## Post-first-users
- [ ] Stripe payment link ($39 / $229 / enterprise)
- [] Parse emails
- [] caching to reduce DB calls on POST /webhook hot path (auth, hmac)
- [] graceful shutdown strategy for worker
- [] .expect("") in worker -- this is an incident waiting to happen
- [] don't log destination URL 
- [] DLQ replay
- [] Idempotency-Key on /webhook
- [] Add internal observability and metrics
- [] Expose customer-facing tenant /metrics and simple dash
- [] Tenant ID prefix format (t_<nanoid>)

Explicitly deferring: Stripe, SSRF, rate limiting, strong ID types, tenant-id format change (UUID → t_<nanoid>). None of these block the demo or first user conversations.