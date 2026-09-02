# HeraldQ observability

One page: what exists, where it lives, and the one gap that matters.

## The pipeline

```
api (9464)  ──┐                         ┌─ alerts      alerts/heraldq.rules.yml (single source)
              ├─ /metrics ─ scraped by ─┤
worker(9465) ─┘                         └─ dashboards  observability/grafana/dashboards/heraldq.json
                                                       (Prometheus for rates/gauges,
                                                        Postgres for per-tenant forensics)
```

- **Emission** — `metrics::` macros throughout `src/`. No-ops unless
  `METRICS_PORT` is set; the api and worker are separate processes, so each
  exports only its own metrics (inventory table in ARCHITECTURE.md).
- **Scrape/eval (local)** — `bash observability/prometheus.sh` →
  http://localhost:9090, loads `prometheus/prometheus.yml`, evaluates the
  shared rule file. No Alertmanager locally on purpose: firing rules show
  at http://localhost:9090/alerts, which is enough to prove a rule works.
- **Display (local)** — `bash observability/grafana.sh` →
  http://localhost:3001/d/heraldq-overview.
- **Alerting** — exactly one home: `alerts/heraldq.rules.yml`. Never add
  Grafana-managed SQL rules next to it; the last time both existed they
  disagreed with each other (see the tombstone in
  `grafana/provisioning/alerting/delete-legacy-rules.yml`).

## Production wiring — the gap that matters

**As of writing, production exports nothing.** Railway has no
`METRICS_PORT` on either service, so every boot logs "Prometheus exporter
disabled" and all 11 alert rules are blind outside a developer laptop.
`HeraldQTargetDown` cannot save you here: it fires on a target that
*exists but won't scrape* — no target at all means no signal, which is
exactly the failure.

To close it:

1. **App side** (enables `/metrics`, no code): set `METRICS_PORT=9464` on
   the api service and `METRICS_PORT=9465` on the worker. The exporter
   binds `::` (dual-stack) specifically for Railway's IPv6-only private
   network.
2. **Exposure**: a scraper outside Railway needs a public path — either a
   Railway public TCP domain per service (e.g. `metrics-api.heraldq.com`),
   or remote_write from a tiny in-Railway Prometheus to Grafana Cloud.
   Prefer the public-domain scrape: fewer moving parts, same firewalling
   story as the api itself (and the endpoint is read-only, unauthenticated
   by design — it carries counters, never payloads or keys).
3. **Grafana Cloud**: add the two scrape targets with job names
   `heraldq-api` and `heraldq-worker` — the names are load-bearing,
   `HeraldQTargetDown` matches `up{job=~"heraldq-.*"}` — and load
   `alerts/heraldq.rules.yml` into the ruler. Wire a contact point
   (`grafana/provisioning/alerting/contactpoints.yml.example` is the
   starting point).
4. **Verify**: `up{job=~"heraldq-.*"}` == 1 for both, and the Billing
   health row on the dashboard shows zeros, not empties.

## Dashboard map

| Row | Answers |
|---|---|
| API — front door | Is intake up, fast, and not 5xxing? |
| Delivery pipeline | Are webhooks landing, how fast, and did any DLQ write fail? |
| Queue health | Are workers keeping up with intake? |
| Delivery forensics (SQL) | Which tenants/endpoints failed, and how? |
| Housekeeping | Is the retention sweeper doing its job? |
| Intake & signup | Why are we rejecting, and is the front door converting? |
| Billing health | Is anyone paying without receiving? |

## Conventions

- Prefix `heraldq_`. Outcome counters are plural with an `outcome` label
  (`heraldq_webhooks_delivered_total`-style); everything else singular.
  One historical quirk (`webhook_delivery_duration` vs `webhooks_delivered`)
  is documented in ARCHITECTURE.md, not churned.
- Labels are cardinality-disciplined: `path` is the route template,
  `method` is a closed set. The tripwire tests live in
  `tests/api_metrics.rs`.
- New metric → three places in the same PR: emission, a panel here if
  actionable, an alert only if a human must act. Counters that merely
  exist are dashboard material, not alert material.
- Panels referencing not-yet-merged metrics render empty, never broken —
  merge order for the current batch: #31 (billing metrics) → #32 (intake
  counters) → this dashboard.
