# HeraldQ delivery behavior, retries and limits

What happens to a webhook after `POST /v1/webhook` returns `202`, and the limits
your integration should be built around.

## Delivery guarantee: at least once

HeraldQ guarantees your endpoint receives every accepted webhook **at least
once** — not exactly once. In rare cases (for example, your endpoint accepts a
delivery but our acknowledgment write fails) the same webhook is delivered
again.

**Build your handler to be idempotent.** Every delivery attempt for the same
webhook carries the same `Idempotency-Key` header (the webhook `id` returned at
enqueue time). Deduplicate on it: if you have already processed that key,
return `200` and skip your side effects.

## Retry schedule

A delivery attempt is judged by your endpoint's response:

| Your response | What we do |
|---|---|
| `2xx` | Success — recorded, visible in `GET /v1/deliveries` |
| `408`, `425`, `429` | Retry with backoff |
| `5xx` | Retry with backoff |
| Timeout (no response within 20s) or connection failure | Retry with backoff |
| Any other status (`3xx`, other `4xx`) | Permanent failure — dead-lettered immediately, no retries |

Redirects are **not** followed; a `3xx` is a permanent failure. Point us
directly at the final URL.

Retryable failures back off exponentially: roughly **2.7s, 7.4s, 20s, 55s,
2.5min, 6.7min, 18min, 50min, 2.25h**, then **~6h** between each remaining
attempt — up to **15 attempts spanning ~34 hours** in total. After the final
failure the webhook is dead-lettered: it appears in `GET /v1/dlq` with the
failure reason and last status code, and `GET /v1/webhook/{id}` reports
`"status": "failed"`.

A webhook that is still retrying reports `"status": "pending"` from
`GET /v1/webhook/{id}`, including the last error and the next attempt time.

## Limits

| Limit | Value | When exceeded |
|---|---|---|
| Enqueue rate | 10 requests/second per account | `429` — retry after a short delay |
| Monthly volume | 50,000 webhooks per calendar month | `429` with a quota message |
| Payload size | 2 MB per webhook | `413` |
| Delivery timeout | 20 seconds per attempt (5s to connect) | attempt counts as retryable failure |

## Destination requirements

- **HTTPS only.** Plain `http://` destinations are rejected at enqueue (`400`).
- **Publicly routable hosts only.** Destinations that are — or resolve to —
  private, loopback, link-local or otherwise non-public addresses are rejected
  or dead-lettered.
- **No credentials in the URL.** `https://user:pass@host/...` is rejected.
  Authenticate deliveries by verifying the signature instead (see
  [webhook-signatures](webhook-signatures.md)).

## Dead-letter retention

Dead-lettered webhooks are currently retained indefinitely and are always
available via `GET /v1/dlq`. A retention window may be introduced later; it
will be announced before it applies.
