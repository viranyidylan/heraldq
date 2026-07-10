# HeraldQ integration kit

Single-file, zero-dependency clients you **vendor** — copy the file for your
language into your project and import it. There is no package to install, no
version to track, no lockfile entry. Each file is ~130 lines you can read in
full before trusting it with your API key.

| Language | File | Requires |
|---|---|---|
| Python | [`python/heraldq.py`](python/heraldq.py) | Python 3.8+, stdlib only |
| Node | [`node/heraldq.mjs`](node/heraldq.mjs) | Node 18+ (built-in `fetch`), no deps |

Both provide the same surface:

- `send(destination, payload)` → `{id}` — enqueue a webhook
- `status(id)` — where it is now: pending, delivered, or dead-lettered
- `deliveries(limit, offset)` / `dlq(limit, offset)` / `summary()`
- `replay(id)` — re-enqueue a dead-lettered webhook (`409` if already replayed)
- `verify_signature(secret, header, raw_body)` — for your **receiving**
  endpoint; see [docs/webhook-signatures.md](../docs/webhook-signatures.md)

Non-2xx responses raise/reject with `HeraldQError` carrying `status` and `body`.

## Why vendored files instead of published packages

Sending a HeraldQ webhook is one authenticated POST — a published SDK package
adds a supply-chain dependency and a maintenance treadmill (registries,
versioning, changelogs, dependabot noise for you) to save one `fetch` call.
The part that genuinely deserves shared code is signature verification, where
subtle mistakes (non-constant-time comparison, verifying re-serialized JSON
instead of raw bytes) are security bugs — so that's what these files
concentrate on getting right, pinned to the same test vector the server's own
test suite asserts (`src/webhook/dispatch.rs`).

If a customer asks for a language we don't cover, the signature spec in
[docs/webhook-signatures.md](../docs/webhook-signatures.md) plus either file
as a reference implementation is a 30-minute port.

## Self-tests

Each file checks itself against the pinned cross-language signature vector:

```sh
python3 sdk/python/heraldq.py
node sdk/node/heraldq.mjs
```
