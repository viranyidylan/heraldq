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
