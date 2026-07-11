//! Process-wide observability wiring, shared by the api and worker binaries.
//!
//! Two knobs, both env-driven so deploys configure them without rebuilds:
//! - `LOG_FORMAT=json` switches logs to one-JSON-object-per-line for log
//!   aggregators; anything else keeps the human-readable format.
//! - `METRICS_PORT=<port>` exposes Prometheus metrics on `0.0.0.0:<port>/metrics`.
//!   Unset, the exporter is skipped and every `metrics::` macro in the code
//!   becomes a no-op — safe for local dev, loud enough in the logs to notice
//!   in production. Each binary is its own process, so give each its own port.

use metrics_exporter_prometheus::PrometheusBuilder;
use sqlx::PgPool;
use std::time::Duration;
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
    #[error("failed to install Prometheus exporter: {0}")]
    Exporter(#[from] metrics_exporter_prometheus::BuildError),
}

/// Install the Prometheus exporter if `METRICS_PORT` is set.
///
/// Must run inside the Tokio runtime (the exporter spawns its HTTP listener
/// as a task). A bad port or a failed bind is a hard error: a production
/// process that *thinks* it is exporting metrics but isn't is exactly the
/// failure mode this module exists to prevent.
pub fn init_metrics() -> Result<(), TelemetryError> {
    let Ok(raw_port) = std::env::var("METRICS_PORT") else {
        warn!("METRICS_PORT not set — Prometheus exporter disabled, metrics are no-ops");
        return Ok(());
    };

    let port: u16 = raw_port
        .parse()
        .map_err(|_| TelemetryError::BadPort(raw_port))?;

    PrometheusBuilder::new()
        .with_http_listener(([0, 0, 0, 0], port))
        .install()?;

    info!(port, "Prometheus exporter listening on /metrics");
    Ok(())
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
