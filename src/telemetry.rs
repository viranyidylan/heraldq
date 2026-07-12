//! Process-wide observability wiring, shared by the api and worker binaries.
//!
//! Two knobs, both env-driven so deploys configure them without rebuilds:
//! - `LOG_FORMAT=json` switches logs to one-JSON-object-per-line for log
//!   aggregators; anything else keeps the human-readable format.
//! - `METRICS_PORT=<port>` exposes Prometheus metrics on `[::]:<port>/metrics`.
//!   Unset, the exporter is skipped and every `metrics::` macro in the code
//!   becomes a no-op — safe for local dev, loud enough in the logs to notice
//!   in production. Each binary is its own process, so give each its own port.
//! - `METRICS_BIND=<ip>` overrides the metrics bind address (default `::`).
//!   Rarely needed; see `DEFAULT_METRICS_BIND`.

use axum::{
    extract::{MatchedPath, Request},
    http::Method,
    middleware::Next,
    response::Response,
};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};
use sqlx::PgPool;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

/// Initialize the global tracing subscriber. Call once, first thing in main.
pub fn init_tracing() {
    let filter = EnvFilter::from_default_env();
    let use_json = std::env::var("LOG_FORMAT").is_ok_and(|v| v == "json");

    if use_json {
        // flatten_event lifts fields like tenant_id to top-level JSON keys,
        // so aggregators can index them without unwrapping a nested object.
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .flatten_event(true)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    #[error("METRICS_PORT is not a valid port number: {0}")]
    BadPort(String),
    #[error("METRICS_BIND is not a valid IP address: {0}")]
    BadBind(String),
    #[error("failed to install Prometheus exporter: {0}")]
    Exporter(#[from] metrics_exporter_prometheus::BuildError),
}

/// Default bind address for the metrics listener: `::` (dual-stack), not
/// `0.0.0.0`.
///
/// Railway's private network is IPv6-only — `<service>.railway.internal`
/// resolves to AAAA records only, so a listener bound to `0.0.0.0` accepts
/// nothing from it and a collector can never scrape us. On Linux, `::` still
/// accepts IPv4 (the kernel default is `net.ipv6.bindv6only=0`, and the
/// exporter binds with plain `std::net::TcpListener`, which does not set
/// `IPV6_V6ONLY`), so `curl localhost:9464/metrics` keeps working unchanged.
///
/// Override with `METRICS_BIND=0.0.0.0` in an environment with IPv6 disabled at
/// the netns level, where binding `::` would fail outright.
const DEFAULT_METRICS_BIND: &str = "::";

/// API latency buckets, in seconds. The API only validates and enqueues, so
/// the interesting range is sub-millisecond to ~1s; anything past that is
/// already an incident and lands in `+Inf`.
const API_LATENCY_BUCKETS: &[f64] = &[
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
];

/// Delivery buckets span a wider range: these are outbound calls to arbitrary
/// customer endpoints, bounded by the dispatch timeout rather than by us.
const DELIVERY_BUCKETS: &[f64] = &[0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0];

/// Install the Prometheus exporter if `METRICS_PORT` is set.
///
/// Must run inside the Tokio runtime (the exporter spawns its HTTP listener
/// as a task). A bad port or a failed bind is a hard error: a production
/// process that *thinks* it is exporting metrics but isn't is exactly the
/// failure mode this module exists to prevent.
///
/// Both histograms get explicit buckets. Without them the exporter renders
/// histograms as Prometheus *summaries* — pre-computed quantiles that cannot
/// be aggregated across instances, so a fleet-wide p99 becomes impossible the
/// moment we run more than one replica. Buckets are the aggregatable form, and
/// they are what `histogram_quantile()` in the alert rules requires.
pub fn init_metrics() -> Result<(), TelemetryError> {
    let Ok(raw_port) = std::env::var("METRICS_PORT") else {
        warn!("METRICS_PORT not set — Prometheus exporter disabled, metrics are no-ops");
        return Ok(());
    };

    let port: u16 = raw_port
        .parse()
        .map_err(|_| TelemetryError::BadPort(raw_port))?;

    let raw_bind =
        std::env::var("METRICS_BIND").unwrap_or_else(|_| DEFAULT_METRICS_BIND.to_owned());
    let host: IpAddr = raw_bind
        .parse()
        .map_err(|_| TelemetryError::BadBind(raw_bind.clone()))?;
    let listen = SocketAddr::new(host, port);

    PrometheusBuilder::new()
        .with_http_listener(listen)
        .set_buckets_for_metric(
            Matcher::Full("heraldq_http_request_duration_seconds".to_owned()),
            API_LATENCY_BUCKETS,
        )?
        .set_buckets_for_metric(
            Matcher::Full("heraldq_webhook_delivery_duration_seconds".to_owned()),
            DELIVERY_BUCKETS,
        )?
        .install()?;

    info!(%listen, "Prometheus exporter listening on /metrics");
    Ok(())
}

/// Label value for requests that matched no route. Without this, 404 floods
/// (scanners probing random paths) would mint an unbounded number of time
/// series — the same cardinality blowup `MatchedPath` exists to prevent.
const UNMATCHED_PATH: &str = "unknown";

/// Label value for methods outside the standard set.
const OTHER_METHOD: &str = "other";

/// Collapse the request method onto a closed set.
///
/// `http::Method` is *not* an enum of the seven verbs — it has an
/// `ExtensionInline` variant that accepts any valid HTTP token, so `PROPFIND`,
/// `MOO`, or any junk a scanner invents parses fine and reaches this middleware.
/// Calling `.to_string()` on it would put caller-controlled input straight into
/// a Prometheus label: one permanent time series per token, forever. Same
/// cardinality hazard `MatchedPath` guards against on `path`, one label over.
///
/// Returning `&'static str` is the point — the type makes it impossible to leak
/// a request-derived string into the label by accident.
fn method_label(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::PATCH => "PATCH",
        Method::DELETE => "DELETE",
        Method::HEAD => "HEAD",
        Method::OPTIONS => "OPTIONS",
        _ => OTHER_METHOD,
    }
}

/// Axum middleware exporting the API's RED metrics:
///
/// - `heraldq_http_requests_total{method,path,status}` — rate and error rate.
/// - `heraldq_http_request_duration_seconds{method,path}` — latency.
///
/// The `path` label is the *route template* (`/v1/webhook/{id}`), never the
/// concrete URI. Labelling by URI would mint one Prometheus time series per
/// webhook ULID and melt the scrape target; `MatchedPath` is what axum stashes
/// in the request extensions after routing, and it is the bounded version.
///
/// Duration deliberately carries no `status` label: p99 latency per endpoint is
/// the useful cut, and every extra label multiplies the series count rather
/// than adding to it.
pub async fn track_metrics(req: Request, next: Next) -> Response {
    // Read what we need before `next.run` takes ownership of the request.
    let method = method_label(req.method());
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| UNMATCHED_PATH.to_owned());

    let start = Instant::now();
    let response = next.run(req).await;
    // Prometheus convention is base units — seconds, not millis.
    let latency = start.elapsed().as_secs_f64();

    let status = response.status().as_u16().to_string();

    metrics::counter!(
        "heraldq_http_requests_total",
        "method" => method,
        "path" => path.clone(),
        "status" => status,
    )
    .increment(1);

    metrics::histogram!(
        "heraldq_http_request_duration_seconds",
        "method" => method,
        "path" => path,
    )
    .record(latency);

    response
}

/// How often the backpressure gauges are refreshed. Prometheus scrapes
/// default to 15s–60s, so polling faster than this buys nothing.
const QUEUE_POLL_INTERVAL: Duration = Duration::from_secs(15);

/// Spawn a background task that polls graphile's job table and exports
/// queue-backpressure gauges:
///
/// - `heraldq_queue_depth` — jobs that are due (`run_at <= now()`) and
///   unclaimed. Scheduled retries with future `run_at` are excluded (they
///   are waiting on backoff, not on worker capacity), as are permanently
///   exhausted jobs (`attempts = max_attempts`) — graphile leaves those in
///   the table as tombstones, and counting them would peg both gauges high
///   forever.
/// - `heraldq_queue_oldest_job_age_seconds` — how long the job at the head
///   of the line has been waiting. This is the true backpressure signal:
///   depth can spike harmlessly if workers drain it, but age climbing means
///   we are falling behind.
///
/// A failed poll logs and skips the tick rather than exiting — a dead poller
/// would freeze the gauges at their last value and lie to every alert built
/// on them. Holds one pool connection per tick; accounted for in the worker
/// pool's headroom.
pub fn spawn_queue_depth_poller(pool: PgPool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(QUEUE_POLL_INTERVAL);
        loop {
            tick.tick().await;

            // Reads the public graphile_worker.jobs view — coupled to
            // graphile internals; re-verify on upgrades (same caveat as
            // db::jobs::find_by_webhook_id).
            let row = sqlx::query!(
                r#"SELECT count(*) AS "depth!",
                    COALESCE(EXTRACT(EPOCH FROM now() - min(run_at)), 0)::float8 AS "oldest_secs!"
                FROM graphile_worker.jobs
                WHERE run_at <= now() AND locked_at IS NULL
                    AND attempts < max_attempts"#
            )
            .fetch_one(&pool)
            .await;

            match row {
                Ok(r) => {
                    metrics::gauge!("heraldq_queue_depth").set(r.depth as f64);
                    metrics::gauge!("heraldq_queue_oldest_job_age_seconds").set(r.oldest_secs);
                }
                Err(e) => warn!(error = ?e, "queue backpressure poll failed, skipping tick"),
            }
        }
    });
}
