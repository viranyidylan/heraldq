//! HTTP surface.
//!
//! This module owns *composition* only — which routes exist, in what order
//! the middleware stack wraps them, and which of them are reachable from a
//! browser. Handler bodies live in the submodules, grouped by the resource
//! they act on:
//!
//! | module     | routes |
//! |------------|--------|
//! | `accounts` | `POST /v1/signup` |
//! | `keys`     | `GET /v1/keys`, `POST /v1/keys/rotate`, `POST /v1/keys/recover` |
//! | `webhooks` | `POST /v1/webhook`, `GET /v1/webhook/{id}`, `POST /v1/mock` |
//! | `dlq`      | `GET /v1/dlq`, `POST /v1/dlq/{id}/replay` |
//! | `stats`    | `GET /v1/deliveries`, `GET /v1/summary`, `GET /v1/dashboard` |
//! | `billing`  | `POST /webhooks/paddle` |
//!
//! Handlers are `pub(super)` — visible to this module for registration, and
//! to nothing else. The router is the only way in.

mod accounts;
mod billing;
mod dlq;
mod keys;
mod stats;
mod webhooks;

use axum::{
    Json, Router,
    extract::{self, FromRef, State},
    middleware,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tower::ServiceBuilder;
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::{DefaultOnResponse, TraceLayer},
};
use tracing::Level;

use crate::{AppState, telemetry};

impl FromRef<AppState> for PgPool {
    fn from_ref(app_state: &AppState) -> PgPool {
        app_state.pg_pool.clone()
    }
}

#[derive(Deserialize, Serialize)]
struct HealthCheckResponse {
    status: i32,
    message: String,
}

pub fn build(state: AppState) -> Router {
    // CORS is scoped to the routes with browser callers: the marketing
    // site's signup form and (future) lost-key recovery form. Every other
    // endpoint authenticates by API key or HMAC and has no browser callers,
    // so it gets no CORS at all (browsers block the response, which is
    // exactly right).
    //
    // `route_layer` wraps exactly the routes registered *before* it, so
    // these two are declared here rather than in their own modules' route
    // lists — the CORS boundary is a composition decision and belongs at
    // the composition point, where it is visible.
    let browser_facing = Router::new()
        .route("/signup", post(accounts::signup))
        .route("/keys/recover", post(keys::recover_key))
        .route_layer(cors_for_signup(&state.cors_allowed_origins));

    let v1 = browser_facing
        .merge(keys::routes())
        .merge(webhooks::routes())
        .merge(dlq::routes())
        .merge(stats::routes());

    Router::new()
        .route("/ping", get(ping))
        .route("/health", get(health))
        // Paddle → us; sits outside /v1 because it authenticates by HMAC
        // signature, not by tenant API key.
        .merge(billing::routes())
        .nest("/v1", v1)
        // ServiceBuilder applies top-down: assign a request id first, then
        // open the trace span (so the id is visible inside it), then copy the
        // id onto the response so a customer can quote it back at us.
        .layer(
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(
                    TraceLayer::new_for_http()
                        .make_span_with(|req: &extract::Request| {
                            let request_id = req
                                .headers()
                                .get("x-request-id")
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("");
                            tracing::info_span!(
                                "request",
                                method = %req.method(),
                                path = %req.uri().path(),
                                request_id,
                            )
                        })
                        // access log: one INFO line per response, with latency
                        .on_response(DefaultOnResponse::new().level(Level::INFO)),
                )
                // Inside the trace span so failures are attributable, and
                // wrapping the handlers so the histogram times handler work.
                .layer(middleware::from_fn(telemetry::track_metrics))
                .layer(PropagateRequestIdLayer::x_request_id()),
        )
        .with_state(state)
}

fn cors_for_signup(origins: &[String]) -> tower_http::cors::CorsLayer {
    use tower_http::cors::{AllowOrigin, CorsLayer};

    let allowed: Vec<_> = origins
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect::<Vec<http::HeaderValue>>();

    CorsLayer::new()
        .allow_methods([axum::http::Method::POST])
        .allow_headers([axum::http::header::CONTENT_TYPE])
        .allow_origin(AllowOrigin::list(allowed))
}

async fn ping(State(_state): State<AppState>) -> Result<&'static str, axum::http::StatusCode> {
    Ok("pong")
}

async fn health() -> Json<HealthCheckResponse> {
    Json(HealthCheckResponse {
        status: 200,
        message: "healthy".into(),
    })
}

/// Shared `?limit=&offset=` query for the two listing endpoints (`dlq`,
/// `deliveries`). Private to `routes`, which makes it visible to every
/// submodule below it and to nothing above.
#[derive(Deserialize)]
struct Pagination {
    limit: Option<i64>,
    offset: Option<i64>,
}

fn clamp_pagination(value: Option<i64>, default: i64, max: i64) -> i64 {
    value.unwrap_or(default).clamp(0, max)
}
