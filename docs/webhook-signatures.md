# Verifying HeraldQ webhook signatures

Every webhook HeraldQ delivers to your endpoint is signed with a secret unique to your
account, so you can prove the request really came from HeraldQ and wasn't tampered with
in transit. Verify the signature on every delivery before trusting the payload.

## Your signing secret

Your `hmac_secret` is returned **once**, in the JSON response when you sign up. Store it
somewhere safe (a secrets manager or environment variable — not your codebase).

> **Important:** the secret is used *as the literal string you received*. Do **not**
> hex-decode it before using it as the HMAC key.

## The signature header

Each delivery carries two headers:

```
HeraldQ-Signature: t=1720000000,v1=5257a869e7ecebeda32affa62cdca3fa51cad7e77a0e56ff536d0ce8e108d8bd
Idempotency-Key: 01J1XAMPLE0ULID0000000000
```

- `t` — Unix timestamp (seconds, UTC) of when HeraldQ signed this delivery attempt.
- `v1` — hex-encoded HMAC-SHA256 signature (see below).

## How the signature is computed

```
signed_content = "{t}" + "." + raw_request_body
signature      = hex( HMAC_SHA256( key = your_hmac_secret_string, message = signed_content ) )
```

The body is signed **byte-for-byte as received**. Read the raw request body and verify
against that — do not parse the JSON and re-serialize it, because key ordering and
whitespace differences will change the bytes and the signature will not match.

## Verification steps

1. Read the raw request body as bytes.
2. Parse `t` and `v1` out of the `HeraldQ-Signature` header.
3. Reject if `t` is more than **5 minutes** from your current time (replay protection).
4. Compute `HMAC_SHA256(secret, "{t}." + raw_body)` and hex-encode it.
5. Compare your result to `v1` using a **constant-time comparison**
   (`hmac.compare_digest` in Python, `crypto.timingSafeEqual` in Node — never `==`).
6. Reject with a 401 if anything fails; process the webhook only if all checks pass.

### Example (Python)

```python
import hashlib, hmac, time

def verify(secret: str, sig_header: str, raw_body: bytes, tolerance_secs: int = 300) -> bool:
    parts = dict(p.split("=", 1) for p in sig_header.split(","))
    timestamp, expected = parts["t"], parts["v1"]

    if abs(time.time() - int(timestamp)) > tolerance_secs:
        return False

    signed_content = timestamp.encode() + b"." + raw_body
    computed = hmac.new(secret.encode(), signed_content, hashlib.sha256).hexdigest()
    return hmac.compare_digest(computed, expected)
```

### Example (Node)

```js
const crypto = require("node:crypto");

function verify(secret, sigHeader, rawBody, toleranceSecs = 300) {
  const parts = Object.fromEntries(sigHeader.split(",").map((p) => p.split("=", 2)));
  const { t, v1 } = parts;

  if (Math.abs(Date.now() / 1000 - Number(t)) > toleranceSecs) return false;

  const computed = crypto
    .createHmac("sha256", secret)
    .update(`${t}.`)
    .update(rawBody) // the raw Buffer, not a re-serialized object
    .digest("hex");

  return crypto.timingSafeEqual(Buffer.from(computed), Buffer.from(v1));
}
```

## Delivery semantics

- **At-least-once.** HeraldQ retries failed deliveries with exponential backoff (up to 15
  attempts over ~9 hours). In rare cases a webhook you already processed may be
  delivered again — deduplicate using the `Idempotency-Key` header, which is stable
  across every attempt of the same webhook.
- **Each attempt is signed fresh.** Retries carry a new `t` and a new signature, so the
  replay-window check in step 3 won't reject legitimate retries.
- **Respond fast.** Return a `2xx` as soon as you've durably accepted the webhook
  (HeraldQ times out at 20 seconds). Do slow processing after responding, not before.
- `408`, `425`, `429`, and `5xx` responses are retried; any other non-`2xx` response is
  treated as a permanent failure and the webhook is dead-lettered.
