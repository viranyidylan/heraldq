"""HeraldQ client + webhook signature verifier. Zero dependencies (stdlib only).

This file is meant to be VENDORED: copy it into your project and import it.
There is no package to install and nothing to keep in sync beyond this file.

Sending:

    from heraldq import HeraldQ

    hq = HeraldQ(api_key="sliq_...", base_url="https://api.example.com")
    accepted = hq.send("https://your-receiver.example.com/hook",
                       {"event": "order.created", "amount": 12999})
    print(accepted["id"])            # poll this
    print(hq.status(accepted["id"]))

Receiving (in your webhook handler — full details in docs/webhook-signatures.md):

    from heraldq import verify_signature

    if not verify_signature(secret, request.headers["HeraldQ-Signature"], request.raw_body):
        return 401
"""

from __future__ import annotations

import hashlib
import hmac
import json
import time
import urllib.error
import urllib.parse
import urllib.request

DEFAULT_TIMEOUT_SECS = 30


class HeraldQError(Exception):
    """Non-2xx API response. `status` is the HTTP code, `body` the response text."""

    def __init__(self, status: int, body: str):
        super().__init__(f"HeraldQ API returned {status}: {body}")
        self.status = status
        self.body = body


def verify_signature(
    secret: str, signature_header: str, raw_body: bytes, tolerance_secs: int = 300
) -> bool:
    """Verify a `HeraldQ-Signature: t=...,v1=...` header against the RAW request body.

    Pass the body bytes exactly as received — parsing and re-serializing the
    JSON changes the bytes and the signature will not match. Stale timestamps
    (outside `tolerance_secs`) are rejected to block replayed captures.
    """
    try:
        parts = dict(p.split("=", 1) for p in signature_header.split(","))
        timestamp, expected = parts["t"], parts["v1"]
    except (ValueError, KeyError):
        return False

    try:
        if abs(time.time() - int(timestamp)) > tolerance_secs:
            return False
    except ValueError:
        return False

    signed_content = timestamp.encode() + b"." + raw_body
    computed = hmac.new(secret.encode(), signed_content, hashlib.sha256).hexdigest()
    # compare_digest, never ==: a plain comparison leaks how many leading
    # characters matched through response timing.
    return hmac.compare_digest(computed, expected)


class HeraldQ:
    def __init__(
        self, api_key: str, base_url: str, timeout_secs: float = DEFAULT_TIMEOUT_SECS
    ):
        self._api_key = api_key
        self._base_url = base_url.rstrip("/")
        self._timeout = timeout_secs

    def send(self, destination: str, payload) -> dict:
        """Enqueue a webhook for delivery. Returns `{"id": ...}` on 202."""
        return self._request(
            "POST", "/v1/webhook", {"destination": destination, "payload": payload}
        )

    def status(self, webhook_id: str) -> dict:
        """Where a webhook is now: queued, delivered, or dead-lettered."""
        return self._request("GET", f"/v1/webhook/{webhook_id}")

    def deliveries(self, limit: int = 25, offset: int = 0) -> dict:
        return self._request("GET", f"/v1/deliveries?limit={limit}&offset={offset}")

    def dlq(self, limit: int = 25, offset: int = 0) -> dict:
        return self._request("GET", f"/v1/dlq?limit={limit}&offset={offset}")

    def replay(self, webhook_id: str) -> dict:
        """Re-enqueue a dead-lettered webhook. Returns `{"id", "replayed_from"}`.

        409 (already replayed) raises HeraldQError with the earlier replay's
        id in the body.
        """
        return self._request("POST", f"/v1/dlq/{webhook_id}/replay")

    def summary(self) -> dict:
        return self._request("GET", "/v1/summary")

    def _request(self, method: str, path: str, body=None) -> dict:
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(
            self._base_url + path,
            data=data,
            method=method,
            headers={
                "Authorization": f"Bearer {self._api_key}",
                "Content-Type": "application/json",
            },
        )
        try:
            with urllib.request.urlopen(req, timeout=self._timeout) as resp:
                text = resp.read().decode()
        except urllib.error.HTTPError as e:
            raise HeraldQError(e.code, e.read().decode()) from None
        return json.loads(text) if text else {}


if __name__ == "__main__":
    # Self-test against the pinned cross-language signature vector (also
    # asserted server-side in src/webhook/dispatch.rs). Run: python3 heraldq.py
    secret = "whsec_testvector_not_a_real_secret"
    body = b'{"event":"order.created","amount":12999}'
    header = "t=1720000000,v1=9619a809cd234fe3ee491cd2a9450e6532a3abb6eda63369204b8b92da650cd9"

    # tolerance is huge because the vector's timestamp is fixed in the past;
    # real callers keep the 300s default.
    forever = 10**12
    assert verify_signature(secret, header, body, tolerance_secs=forever)
    assert not verify_signature(secret, header, body + b" ", tolerance_secs=forever)
    assert not verify_signature("wrong", header, body, tolerance_secs=forever)
    assert not verify_signature(secret, header, body)  # stale timestamp rejected
    assert not verify_signature(secret, "garbage", body, tolerance_secs=forever)
    print("heraldq.py: all signature self-tests passed")
