/**
 * HeraldQ client + webhook signature verifier. Zero dependencies
 * (built-in fetch and node:crypto; Node 18+).
 *
 * This file is meant to be VENDORED: copy it into your project and import it.
 * There is no package to install and nothing to keep in sync beyond this file.
 *
 * Sending:
 *
 *   import { HeraldQ } from "./heraldq.mjs";
 *
 *   const hq = new HeraldQ({ apiKey: "sliq_...", baseUrl: "https://api.example.com" });
 *   const { id } = await hq.send("https://your-receiver.example.com/hook",
 *                                { event: "order.created", amount: 12999 });
 *   console.log(await hq.status(id));
 *
 * Receiving (in your webhook handler — full details in docs/webhook-signatures.md):
 *
 *   import { verifySignature } from "./heraldq.mjs";
 *
 *   if (!verifySignature(secret, req.headers["heraldq-signature"], rawBodyBuffer)) {
 *     return res.status(401).end();
 *   }
 */

import crypto from "node:crypto";

export class HeraldQError extends Error {
  constructor(status, body) {
    super(`HeraldQ API returned ${status}: ${body}`);
    this.status = status;
    this.body = body;
  }
}

/**
 * Verify a `HeraldQ-Signature: t=...,v1=...` header against the RAW request
 * body (a Buffer — not a parsed-and-restringified object, whose bytes can
 * differ). Stale timestamps outside `toleranceSecs` are rejected to block
 * replayed captures.
 */
export function verifySignature(secret, signatureHeader, rawBody, toleranceSecs = 300) {
  if (typeof signatureHeader !== "string") return false;
  const parts = Object.fromEntries(
    signatureHeader.split(",").map((p) => p.split("=", 2)),
  );
  const { t, v1 } = parts;
  if (!t || !v1) return false;

  if (Math.abs(Date.now() / 1000 - Number(t)) > toleranceSecs) return false;

  const computed = crypto
    .createHmac("sha256", secret)
    .update(`${t}.`)
    .update(rawBody)
    .digest("hex");

  // timingSafeEqual, never ===: a plain comparison leaks how many leading
  // characters matched through response timing. It throws on length
  // mismatch, so guard that first (length is not secret).
  const a = Buffer.from(computed);
  const b = Buffer.from(v1);
  return a.length === b.length && crypto.timingSafeEqual(a, b);
}

export class HeraldQ {
  #apiKey;
  #baseUrl;

  constructor({ apiKey, baseUrl }) {
    this.#apiKey = apiKey;
    this.#baseUrl = baseUrl.replace(/\/+$/, "");
  }

  /** Enqueue a webhook for delivery. Resolves to `{ id }` on 202. */
  send(destination, payload) {
    return this.#request("POST", "/v1/webhook", { destination, payload });
  }

  /** Where a webhook is now: queued, delivered, or dead-lettered. */
  status(webhookId) {
    return this.#request("GET", `/v1/webhook/${webhookId}`);
  }

  deliveries(limit = 25, offset = 0) {
    return this.#request("GET", `/v1/deliveries?limit=${limit}&offset=${offset}`);
  }

  dlq(limit = 25, offset = 0) {
    return this.#request("GET", `/v1/dlq?limit=${limit}&offset=${offset}`);
  }

  /**
   * Re-enqueue a dead-lettered webhook. Resolves to `{ id, replayed_from }`.
   * 409 (already replayed) rejects with HeraldQError carrying the earlier
   * replay's id in the body.
   */
  replay(webhookId) {
    return this.#request("POST", `/v1/dlq/${webhookId}/replay`);
  }

  summary() {
    return this.#request("GET", "/v1/summary");
  }

  async #request(method, path, body) {
    const resp = await fetch(this.#baseUrl + path, {
      method,
      headers: {
        Authorization: `Bearer ${this.#apiKey}`,
        "Content-Type": "application/json",
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await resp.text();
    if (!resp.ok) throw new HeraldQError(resp.status, text);
    return text ? JSON.parse(text) : {};
  }
}

// Self-test against the pinned cross-language signature vector (also
// asserted server-side in src/webhook/dispatch.rs). Run: node heraldq.mjs
if (import.meta.url === `file://${process.argv[1]}`) {
  const secret = "whsec_testvector_not_a_real_secret";
  const body = Buffer.from('{"event":"order.created","amount":12999}');
  const header =
    "t=1720000000,v1=9619a809cd234fe3ee491cd2a9450e6532a3abb6eda63369204b8b92da650cd9";

  // tolerance is huge because the vector's timestamp is fixed in the past;
  // real callers keep the 300s default.
  const forever = 1e12;
  const assert = (cond, msg) => {
    if (!cond) throw new Error(`self-test failed: ${msg}`);
  };
  assert(verifySignature(secret, header, body, forever), "valid signature verifies");
  assert(!verifySignature(secret, header, Buffer.concat([body, Buffer.from(" ")]), forever), "tampered body rejected");
  assert(!verifySignature("wrong", header, body, forever), "wrong secret rejected");
  assert(!verifySignature(secret, header, body), "stale timestamp rejected");
  assert(!verifySignature(secret, "garbage", body, forever), "malformed header rejected");
  console.log("heraldq.mjs: all signature self-tests passed");
}
