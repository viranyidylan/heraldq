# API keys: rotation, listing, and recovery

Keys are shown exactly once — at signup, at rotation, or at recovery.
There is no login and no key retrieval after the fact; storage is the
tenant's responsibility.

## Rotate — `POST /v1/keys/rotate`

Authenticated with the current key. Revokes every active key for the
tenant and issues exactly one replacement, in a single transaction: there
is never a moment with two live keys.

```
curl -X POST https://api.heraldq.com/v1/keys/rotate \
  -H 'authorization: Bearer sliq_...'
# -> 200  { "api_key": "sliq_..." }     (old key is dead from this response on)
```

Rotation is also the incident response for a suspected leak: the response
of one request invalidates the compromised credential.

## Recover — `POST /v1/keys/recover`

For a lost key. The account email plus the HMAC signing secret (shown
next to the key at signup, and required anyway to verify our outbound
webhook signatures) is the proof of account — no email round trip.

```
curl -X POST https://api.heraldq.com/v1/keys/recover \
  -H 'content-type: application/json' \
  -d '{"email":"you@example.com","hmac_secret":"..."}'
# -> 200  { "api_key": "sliq_..." }     (all previous keys revoked)
```

Semantics:

- Wrong secret and unknown email return the same 401 — recovery cannot be
  used to enumerate which addresses have accounts.
- A failed recovery leaves existing keys untouched.
- A successful recovery revokes **every** existing key. A legitimate
  tenant whose key suddenly stopped working has just learned their secret
  leaked — that's the trade, and it's the right one.
- Rate-limited like signup (100/min globally).

## List — `GET /v1/keys`

Authenticated. Key metadata only — ids, names, timestamps, active flag.
Hashes and raw values are never returned.

```
curl -H 'authorization: Bearer sliq_...' https://api.heraldq.com/v1/keys
# -> [{ "id": "...", "name": "rotated", "created_at": "...",
#       "expires_at": null, "is_active": true }, ...]
```
