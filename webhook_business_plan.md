# WEBHOOK DELIVERY AS A SERVICE
## Go-to-Market Battle Plan
*No fluff. No BS. Grounded in real competitor data.*

---

> **The Only Two Modes That Matter: BUILD PRODUCT | TALK TO USERS**

Everything else is procrastination wearing a productive disguise.

---

# The Competitive Landscape — Know Who You're Fighting

This is the single biggest upgrade to your plan. You're not entering a vacuum. There are funded, YC-backed companies already here. Understanding them is the difference between building something clever and building something dead on arrival.

## The Big Three Competitors

| Competitor | What They Do | Pricing | Funding / Traction |
|---|---|---|---|
| **Svix** | Outbound webhooks-as-a-service. Embeddable portal for your customers to manage subscriptions, inspect logs, replay events. | Free: 50K msgs/mo · Pro: $490/mo (1M msgs) · Enterprise: Custom | $13M raised (a16z, YC W21). ~$550K est. revenue. Brex, Lob, Etsy as customers. ~12 employees. |
| **Hookdeck** | Inbound event gateway. Sits between webhook providers and your API. Queuing, filtering, dedup, observability. | Free: 10K events · Team: $39/mo ($3.30/1M events) · Growth: $499/mo (99.999% SLA) | Montreal-based. Strong dev community. Usage-based pricing is very competitive. |
| **Convoy** | Open-source webhook gateway. Both inbound and outbound. Self-hostable. MIT-licensed. | Open source: Free · Cloud Pro: $99/mo · Enterprise: Custom | YC-backed. 3K GitHub stars. Built in Go. Strong in compliance-sensitive verticals. |

## Other Players

- **Webhook Relay** — forwarding/tunneling, from $7.50/mo. Niche: routing to private networks.
- **Hook0** — open-source approach, still small.
- **Codehooks.io** — code-first template approach. Not a managed service.
- **Zapier Webhooks** — no-code, but expensive at scale ($20–50+/mo) and limited customization.

## What the Data Says About the Market

- 85% of the top 100 API companies now offer webhooks, up from 83% in 2023 (Svix State of Webhooks 2024).
- 50% of all organizations use webhooks as an API pattern (Postman 2025 State of the API Report).
- The software dev tools market is $7.4B in 2026, growing at 16% CAGR (Mordor Intelligence).
- Svix describes itself as "SendGrid for webhooks" — a useful framing for how large this category could become.
- Building webhook infra in-house takes 6–12 months for most teams. This is the pain you monetize.

---

# Your Angle — Where You Can Win

You cannot out-feature Svix with a $13M head start. You cannot out-price Hookdeck at $3.30/million events. Here's where you CAN compete:

| Angle | Why It Works | Who Cares |
|---|---|---|
| **Rust performance story** | Svix is Python/Rust hybrid, Convoy is Go, Hookdeck is Node. A pure Rust stack is genuinely differentiated on latency and resource efficiency. | Infra-obsessed devs, cost-sensitive startups |
| **Simpler, cheaper, smaller** | Svix jumps from free to $490/mo. That's a massive gap. Hookdeck's $39 Team tier is better, but still complex. You can own the $29–$99 range. | Indie hackers, solo SaaS founders, early-stage teams |
| **Opinionated simplicity** | Competitors are building platforms. You can build a tool. One API call, one dashboard. Done. | Developers who want to add webhooks in an afternoon, not a sprint |
| **Self-host option** | Convoy has open-source, but it's complex (Go, Kubernetes). A single-binary Rust deployment that "just works" is killer. | Privacy-conscious teams, regulated industries |

*Your pitch is NOT "webhooks as a service." That's Svix's tagline. Your pitch is: "Webhook delivery that's fast, simple, and doesn't cost $490/month."*

---

# Phase 0 — Make It Sellable (1–2 Weeks, Mostly Build)

The product works but can't be sold. You can't sell what can't be onboarded or observed.

## The 5 Things a Paying Customer Needs

1. **GET /status endpoint** — "Is my stuff being delivered?" This is what closes deals. Svix and Hookdeck both have full observability dashboards. You need at minimum a JSON endpoint that shows delivery state.
2. **GET /dlq endpoint** — "What failed and why?" Dead letter queue visibility. Convoy highlights this as a top feature. No one pays for a black box.
3. **Self-serve API key provisioning** — or a manual flow you can do in < 2 minutes. Svix lets you sign up and get an API key in seconds. You need to at least match a frictionless onboarding.
4. **One curl-to-working demo** — not docs, literally one command that proves value. Hookdeck advertises "integration in 30 minutes." You should target "working in 5 minutes."
5. **HMAC payload signing** — Non-negotiable. Svix's 2024 report shows 17% YoY increase in adoption of HMAC-SHA256 signatures. Every serious buyer will ask about this.

*The restructure (your PROJECT_RESTRUCTURE.md) is engineering-correct. Do it in parallel, don't let it block demos. Timebox it to 2 sessions.*

---

# Phase 1 — First Customer (Weeks 2–4, 80% Talk)

**Target:** one person paying anything ($20–$100/mo) who isn't your friend.

## Where to Find Them (Priority Order)

1. **Indie Hackers** — post "I built a webhook delivery service in Rust, looking for beta testers." The Rust angle is catnip for this audience. Developers building SaaS are your most natural first users.
2. **Twitter/X** — search "webhook failed", "webhook delivery unreliable", "retry webhook." Reply to real pain. Don't pitch, help. Then mention your thing.
3. **Hacker News** — a Show HN post with the Rust angle will get attention. "Show HN: I built a webhook delivery service in Rust" is a high-signal title. Time it AFTER you have the curl demo working.
4. **r/SaaS, r/startups, r/rust** — the Rust community specifically will amplify a well-built Rust project.
5. **YC Startup Directory / LinkedIn** — DM small SaaS founders in their first year. They're the ones building webhook infra for the first time.

## The Pitch

*"Your webhooks fail silently. Mine don't — retries, dead letter queue, delivery status. Built in Rust. Starts at $29/month."*

Note: the original plan said "$X/month." Don't leave price ambiguous. Name a number. $29 is low enough to be an impulse buy for a SaaS founder and clearly undercuts Svix's $490 pro tier.

## What You're Learning in Phase 1

- What do they actually care about? (Retries? Visibility? Speed? All three?)
- What would make them NOT use you? (Price? Trust? Missing an integration? No dashboard?)
- What do they call this problem? (That's your marketing copy, verbatim.)
- Would they pay more for a self-hosted binary? (This is your enterprise play.)

*First customer milestone: one person hands you a card number or pays a Stripe invoice. That's it.*

---

# Phase 2 — $1K MRR (Months 1–3, 50/50 Build/Talk)

$1K MRR = ~15–20 customers at $49–69/mo, or a mix of tiers. Very achievable for a developer tool if the product solves a real problem.

## Revised Pricing — Grounded in Competitor Data

The original plan's pricing ($49/$149/$499) was reasonable but needs to be positioned against real alternatives:

| Tier | Price | Includes | Positioned Against |
|---|---|---|---|
| **Starter** | $29/mo | 25K deliveries/mo, 3-day log retention, 1 API key, HMAC signing | Svix Free (50K msgs but no support, no SLA) · Hookdeck Free (10K events) |
| **Pro** | $79/mo | 250K deliveries/mo, 14-day retention, 5 API keys, DLQ viewer, priority support | Svix's $490 gap · Hookdeck $39 Team (but simpler) |
| **Growth** | $199/mo | 2M deliveries/mo, 30-day retention, unlimited keys, webhook replay, rate limiting | Convoy Cloud $99 (but managed) · Hookdeck $499 Growth |
| **Self-Hosted** | $499/mo | Single Rust binary, unlimited volume, your infra, email support | Convoy open-source (but complex) · Svix Enterprise (call for pricing) |

**Key insight:** Svix has a massive pricing gap between free and $490/mo. Hookdeck is cheaper but complex. You own the "simple + affordable" lane from $29–$199. If nobody pushes back on price, you're too cheap.

## What to Build (In Order of Customer Value)

1. GET /status + GET /dlq — visibility is what closes deals and prevents churn
2. HMAC-SHA256 payload signing — table stakes, Svix report shows this is the #1 growing best practice
3. Structured logging / tracing — your own ops sanity + customer trust
4. Rate limiting — required before you let more than a handful of customers in
5. Stripe integration + dead simple pricing page — keep it to one page, three tiers
6. Basic dashboard UI — even minimal, it builds trust over raw JSON endpoints

## What to Talk About Every Week

- 3–5 conversations with potential or existing customers.
- Ask: "What would make you tell a colleague about this?" — that's your referral unlock.
- Ask: "What almost stopped you from signing up?" — that's your conversion leak.
- Ask: "What are you using right now for webhooks?" — that tells you if you're replacing DIY, a competitor, or nothing.

---

# Phase 3 — $5K MRR (Months 3–6, Product Tightens)

$5K MRR = ~35–50 customers across tiers, or fewer with some Growth/Self-Hosted plans.

## What Unlocks This Phase

1. **Referrals** — developers talk to developers. One happy customer in a Slack community is worth 10 cold DMs. Svix's G2 reviews show that word-of-mouth from engineers is their primary growth channel.
2. **One killer content piece** — "Why Webhooks Fail (and How to Fix It)" posted on HN / Indie Hackers. Svix literally wrote the State of Webhooks report and it drives massive inbound. You don't need a blog. You need ONE authoritative post.
3. **Integrations** — "Works alongside Stripe/Clerk/Resend" makes the pitch obvious. Svix highlights Stripe and Shopify integrations prominently.
4. **Per-tenant usage dashboards** — once you have multiple customers, they want their data, not aggregate. Svix's embeddable portal is one of their strongest features. You don't need to match it, but you need per-customer visibility.
5. **The self-hosted play** — this is your unique wedge. A single Rust binary that deploys in 60 seconds is something no competitor offers cleanly. Convoy is open-source but requires Go + Kubernetes knowledge. This can be your enterprise differentiator.

**Structural milestone:** you should be making product decisions from real usage patterns, not guesses.

---

# Anti-Patterns — What Will Kill You

| Looks Productive | Actually Is | What to Do Instead |
|---|---|---|
| Polishing a dashboard UI for a week | Avoiding talking to people | Ship ugly, get feedback, iterate |
| Full PROJECT_RESTRUCTURE before any GTM | Perfectionism | Timebox to 2 sessions, ship in parallel |
| Building rate limiting before customer #3 | Premature scaling | Rate limit manually until it hurts |
| A feature roadmap without customer input | Fiction | Roadmap = what customers asked for this week |
| Studying Svix's architecture for a month | Competitor obsession | Know them, then stop looking. Build. |
| Building a Terraform provider or SDK | Gold-plating | One curl command is enough for 6 months |
| Waiting for the perfect landing page | Avoidance | A README on GitHub + a Stripe link is enough to start |

---

# Your Next 5 Actions (In Order, This Week)

1. **Ship GET /status as a JSON endpoint** — drives a 5-minute demo. This is your "show don't tell" moment.
2. **Add HMAC-SHA256 signing to outbound webhooks** — takes a day, makes you credible to any security-aware buyer.
3. **Write the one-curl demo** — `curl -X POST your-service.com/api/v1/webhooks -H 'Authorization: Bearer demo-key' -d '{...}'` and have it actually deliver a webhook to a test URL.
4. **Post on Indie Hackers this week** — "Built a webhook delivery service in Rust, looking for beta users." Get 5 conversations. Don't sell, listen.
5. **Finish the restructure** — timebox to 2 sessions, then stop. Don't let it be a reason not to ship.

---

# Key Metrics to Track From Day 1

| Metric | Target | Why |
|---|---|---|
| Demo-to-signup rate | > 20% | If people see it and don't sign up, your demo is broken |
| Signup-to-paid rate | > 5% | If they sign up but don't pay, your free tier is too generous or your paid tier isn't compelling |
| Time-to-first-webhook | < 5 min | Hookdeck advertises 30 min. Svix says "minutes." Beat them. |
| Weekly customer conversations | ≥ 3 | Below this, you're building in a vacuum |
| Webhook delivery success rate | > 99.5% | This is your product. If this number is bad, nothing else matters. |
| MRR | Track weekly | The only number that matters for survival |

---

> **The product is good enough to show people today. The thing that will stall you is building in isolation waiting until it's "ready." It's never ready. Ship it.**
