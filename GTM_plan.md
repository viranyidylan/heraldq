

- [x] per tenant hmac secret and signup_codes table
- [x] hmac secret generation logic
- [x] update /signup to require email and invite_code
- [x] dispatch.rs loads the per-tenant hmac secret
- [x] create a script that seeds some invite codes

then 
- [] railway deploy and smoke-test:
 - [] Confirm both Dockerfiles build --release locally.
 - [] Push; set DATABASE_URL on api + worker services.
 - [] Watch logs: migrations ran, both services green.
 - [] Smoke sequence via the deployed URL:
    - 1. POST /signup with a beta code → grab api_key + hmac_secret
    - 2. POST /webhook with bearer api_key, destination = webhook.site URL → 202
    - 3. Check webhook.site received it with valid Sliq-Signature
    - 4. GET /summary → delivered count = 1
- [] Verify: all four above pass against the live Railway URL.

- [] Onboarding / invite / payment flow:
    - [] Invite code
    - [] Trial key
    - [] HMAC generation and storage
- [] One-curl demo that proves value end-to-end: `curl -X POST .../api/v1/webhooks` → delivers to a test URL → visible in `/status`

## Blocking demo
- [ ] Migration: per-tenant hmac_secret + beta_codes table
- [ ] Signup requires invite code, returns api_key + hmac_secret once
- [ ] Dispatch reads hmac_secret per-tenant (drop env var)
- [ ] Seed ~10 beta codes
- [ ] Railway deploy + end-to-end smoke test (signup → webhook → summary)
- [ ] Record 90s Loom curl demo
- [ ] Indie Hackers post + 10 direct outreach DMs

## Post-first-users
- [ ] Stripe payment link ($29 / $79 / $199)
- [ ] SSRF hardening (ValidatedUrl newtype)
- [ ] Rate limiting on /signup and /webhook
- [ ] Tenant ID prefix format (t_<nanoid>)

Explicitly deferring: Stripe, SSRF, rate limiting, strong ID types, tenant-id format change (UUID → t_<nanoid>). None of these block the demo or first user conversations.