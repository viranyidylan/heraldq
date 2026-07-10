# HeraldQ API reference

Base URL: your deployment's API host. All `/v1/*` endpoints except `/v1/signup`
require your API key:

```
Authorization: Bearer sliq_...
```

Prefer not to hand-roll requests? Vendor a single-file client from
[`sdk/`](../sdk/README.md) (Python and Node).

## Quickstart

```sh
# 1. Sign up (invite code required). SAVE THE RESPONSE — the api_key and
#    hmac_secret are shown exactly once.
curl -s -X POST $HERALDQ/v1/signup \
  -H 'Content-Type: application/json' \
  -d '{"invite_code":"...","name":"Acme","email":"ops@acme.test"}'

# 2. Send a webhook
curl -s -X POST $HERALDQ/v1/webhook \
  -H "Authorization: Bearer $API_KEY" -H 'Content-Type: application/json' \
  -d '{"destination":"https://acme.test/hooks","payload":{"event":"order.created"}}'
# -> 202 {"id":"01J..."}

# 3. Follow it
curl -s -H "Authorization: Bearer $API_KEY" $HERALDQ/v1/webhook/01J...
```

## Endpoints

| Method | Path | What it does |
|---|---|---|
| `POST` | `/v1/signup` | Redeem an invite code for a tenant + trial API key. Unauthenticated, rate-limited. |
| `POST` | `/v1/webhook` | Enqueue a webhook. `202 {"id"}`. Body: `{"destination", "payload"}`. |
| `GET` | `/v1/webhook/{id}` | Status of one webhook: pending (with attempt count and next retry), delivered, or dead-lettered. |
| `GET` | `/v1/deliveries?limit&offset` | Successful deliveries, newest first. |
| `GET` | `/v1/dlq?limit&offset` | Dead-lettered webhooks with failure reason; replayed entries carry `replayed_at`/`replayed_as`. |
| `POST` | `/v1/dlq/{id}/replay` | Re-enqueue a dead-lettered webhook as a fresh delivery. `202 {"id", "replayed_from"}`; `409` if already replayed (body names the replay's id). |
| `GET` | `/v1/summary` | Activity counts for your tenant. |
| `GET` | `/v1/dashboard` | The same, as HTML. |
| `GET` | `/health` | Unauthenticated liveness probe. |

## Errors

Errors are plain-text bodies with conventional status codes: `401` bad/expired
key, `404` not yours or doesn't exist, `409` conflict (e.g. already replayed),
`422` malformed body, `429` rate or monthly quota limit, `400` destination
rejected (unroutable, non-HTTPS, or private-network address). Every response
carries an `x-request-id` header — include it when reporting a problem.

## Limits and delivery semantics

- Delivery is **at least once**; deduplicate on the `Idempotency-Key` header.
  Retry schedule, timeouts, and status-code handling:
  [delivery-behavior.md](delivery-behavior.md).
- Verify every delivery's `HeraldQ-Signature`:
  [webhook-signatures.md](webhook-signatures.md).
- 50,000 webhooks per calendar month per tenant; burst rate limits apply
  per-tenant on enqueue and replay.
