# HeraldQ — Backlog

One file, kept current. The **priority ladder** is the source of truth for
what's next; the **matrix** is for planning conversations. When they
disagree, the ladder wins.

- **P0** — blocks revenue or actively risks trust. Do now.
- **P1** — high value, next up after P0.
- **P2** — valuable, unscheduled. Picked up opportunistically.
- **P3** — nice to have; only with slack, or when touching adjacent code.

Impact = revenue/trust/reliability at stake. Size = S (< day), M (days),
L (week+).

## Priority ladder

| # | Pri | Item | Impact | Size | Notes |
|---|-----|------|--------|------|-------|
| 1 | ~~P0~~ done | ~~Merge PR #26 and pin prod deploys to `main`~~ | H | S | Merged `a4bea0e` 2026-08-26; api + worker auto-deployed from main and verified live. |
| 2 | ~~P0~~ done | ~~Set `PADDLE_API_KEY` (sandbox) on the api service~~ | H | S | Set 2026-08-26; repeat-buyer hydration verified end-to-end on prod. Swap key + `PADDLE_API_URL=https://api.paddle.com` at go-live. |
| 3 | P0 | Paddle live cutover | H | S–M | Production client token + price IDs in `signup.html`; prod notification destination; domain approval for heraldq.com checkout; production API key + URL; verify with one real transaction. |
| 4 | P0 | Tier-based quota enforcement | H | M | Pricing promises 5k / 50k / 500k webhooks per month by tier; code enforces one global 50k constant (`MONTHLY_WEBHOOK_LIMIT`). This is the entire point of the billing mirror. |
| 5 | ~~P0~~ done | ~~Prod hygiene: deactivate dead sandbox destination~~ | M | S | Deactivated 2026-08-26 via API update (`active: false`), not dashboard — the earlier "API refuses" note was a failed `delete` attempt. Live destination `ntfset_01m0ztym...` untouched and still active. |
| 6 | P1 | Per-tier rate limits + retention wiring | H | M | Pricing promises 240 / 1.2k / 3k per minute by tier; code has uniform 10 req/s (600/min). Retention columns exist per tenant (migration 0009) — map tier → default retention days at signup/upgrade. |
| 7 | P1 | API key lifecycle: rotate / revoke / lost-key recovery | H | M | No login exists; a lost key is a manual email today. Even minimal "reissue key from verified email" closes the biggest support hole. |
| 8 | P1 | Billing observability | M | S | Alerts cover DLQ/queue/API but nothing Paddle: alert on webhook 500 bursts (Paddle retries exhausting), stub customers unhydrated > 24h, `active` Paddle subs whose tenant is still `free` (drift detector). |
| 9 | P1 | Worker reliability: remove `.expect()` panics, graceful SIGTERM drain | M | S–M | `.expect` in the worker is an incident waiting to happen (GTM list); worker needs the same drain-on-shutdown the API already has. |
| 10 | P1 | Email infrastructure (transactional) | M | M | No outbound email at all today. Unblocks: signup key delivery fallback, payment receipts, quota warnings. |
| 11 | P1 | ARCHITECTURE.md catch-up | L | S | Document paddle module, signup-first flow, CORS, hydration; keep it the authoritative orientation it claims to be. |
| 12 | P2 | **Migrate Railway → AWS** | M | L | Scope sketch below. Migrate when ANY trigger fires, not before: (a) a paying customer is affected by a Railway outage, (b) MRR > $500/mo, or (c) Railway bill > $60/mo. Pre-work done first: Grafana Cloud scraping prod, Postgres backups verified on. |
| 13 | P2 | Hot-path caching (api-key lookup, hmac secret) | M | S–M | Two DB round-trips per enqueue; fine now, first perf wall later. |
| 14 | P2 | `Idempotency-Key` on `POST /v1/webhook` | M | S | At-least-once enqueue makes client retries duplicate deliveries; let clients opt into dedup. |
| 15 | P2 | Encrypt / externally manage HMAC secrets | M | M | Per-tenant secrets sit plaintext in Postgres (GTM list). |
| 16 | P2 | Paddle customer-portal session minting | M | M | Let paying users manage/cancel from our docs site instead of hunting receipt emails. |
| 17 | P2 | Privacy: stop logging destination URLs | L | S | Quick hygiene, on the GTM list. |
| 18 | P3 | Tenant ID format `t_<nanoid>` | L | M | Cosmetic + breaking for existing customers; only worth it with an alias layer. |
| 19 | P3 | Overage / custom-volume billing | L | M | "Email us for enterprise" is fine until someone pays. |

## Impact × complexity matrix

```
                 Small                    Medium                    Large
High      #1 merge+pin deploys      #4 quota enforcement      #12 AWS migration
          #2 PADDLE_API_KEY         #6 rate limits+retention  #7 key lifecycle
          #3 live cutover
          #5 prod hygiene

Medium    #8 billing alerts         #10 email infra
          #9 worker reliability     #15 secret management
          #14 idempotency key       #16 portal sessions
          #17 URL logging

Low       #11 docs catch-up         #18 tenant ID format
                                     #19 overage billing
```

- **Top-left first** (quick wins), **bottom-right never by accident** — items
  there need a deliberate decision, not momentum.
- #12 (AWS) is the one big bet: real value, but only pay it when Railway
  cost or control actually hurts.

## #12 scope sketch — Railway → AWS

Not a commitment; the shape of the work if we pull it:

1. **RDS Postgres** — restore from `pg_dump` (app tables + graphile schema);
   same `DATABASE_URL` contract, zero code change.
2. **ECS Fargate** — two services (`api`, `worker`), same Dockerfiles,
   health checks on `/health`; worker needs no public ingress.
3. **ALB + Cloudflare** — keep Cloudflare DNS, repoint at the ALB;
   TLS terminates at the ALB. `api.heraldq.com` cutover is one DNS change.
4. **Secrets** — SSM Parameter Store / Secrets Manager for `DATABASE_URL`,
   Paddle keys, webhook secret.
5. **Metrics** — Prometheus/Grafana containers are already self-hosted
   (observability/); move alongside, or ship to CloudWatch first and
   decide later. Wire the existing `alerts/heraldq.rules.yml`.
6. **Cutover** — freeze writes, final pg_dump, DNS flip, watch `/health` +
   one webhook e2e. Keep Railway stack warm for a week before deleting.

## Done

- 2026-08-26: Dead sandbox notification destination `ntfset_01m0wvcr...`
  ("local app ngrokked", stale ngrok URL failing every event) deactivated via
  API update; only the live `api.heraldq.com` destination remains active —
  PR #28.
- 2026-08-26: PR #26 merged (`a4bea0e`); api + worker redeployed from main;
  all three billing convergence paths verified on prod (customer event,
  repeat-buyer hydration, signup-time backfill).
- 2026-08-26: Signup-first flow (open signup, non-expiring keys, email
  normalization, CORS for the site) — PR #26.
- 2026-08-26: Paddle billing mirror + tier propagation, stub-customer
  hydration — PR #26.
- 2026-08-26: Site signup + checkout-after-account on heraldq.com.
