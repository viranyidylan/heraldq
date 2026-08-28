use axum::{
    Json, Router,
    body::Bytes,
    extract::{self, FromRef, Path, Query, State},
    http::HeaderMap,
    middleware,
    response::Html,
    routing::{get, post},
};
use chrono::Utc;
use rand::RngExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tower::ServiceBuilder;
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::{DefaultOnResponse, TraceLayer},
};

use tracing::{Level, debug, error, info, warn};

use crate::{
    AppState, WebhookRequest, auth,
    db::stats::WebhookActivitySummary,
    error::AppError,
    paddle, telemetry,
    webhook::{self, ssrf::validate_destination},
};

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
    let cors = cors_for_signup(&state.cors_allowed_origins);

    let v1 = Router::new()
        .route("/signup", post(signup))
        .route("/keys/recover", post(recover_key))
        .route_layer(cors)
        .route("/keys", get(list_keys))
        .route("/keys/rotate", post(rotate_key))
        .route("/webhook", post(enqueue_webhook))
        .route("/webhook/{id}", get(webhook_status))
        .route("/mock", post(mock))
        .route("/dlq", get(dlq))
        .route("/dlq/{id}/replay", post(dlq_replay))
        .route("/deliveries", get(deliveries))
        .route("/summary", get(webhook_activity_summary))
        .route("/dashboard", get(dashboard));

    Router::new()
        .route("/ping", get(ping))
        .route("/health", get(health))
        // Paddle → us; sits outside /v1 because it authenticates by HMAC
        // signature, not by tenant API key.
        .route("/webhooks/paddle", post(paddle_webhook))
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

/// Per-tenant cap on webhooks enqueued per calendar month.
const MONTHLY_WEBHOOK_LIMIT: i64 = 50_000;

async fn ping(State(_state): State<AppState>) -> Result<&'static str, StatusCode> {
    Ok("pong")
}

async fn health() -> Json<HealthCheckResponse> {
    Json(HealthCheckResponse {
        status: 200,
        message: "healthy".into(),
    })
}

#[derive(Serialize)]
struct PaddleWebhookAck {
    received: bool,
    duplicate: bool,
}

/// Receive a Paddle webhook delivery.
///
/// Status codes follow Paddle's contract — only 2xx within 5s counts as
/// delivered, every non-2xx is retried on the same budget — so none of
/// these responses can lose an event that a later attempt could deliver:
///
/// - 503 — no webhook secret configured (deployment issue; loud + retried)
/// - 400 — structurally unusable request (missing signature/body, malformed
///   header, unparseable JSON). Redelivery is byte-identical, so retrying
///   can't help, but it keeps the dashboard's failure log honest.
/// - 401 — HMAC mismatch or stale timestamp. A rotated secret recovers on
///   the next retry after redeploy; a forged request is harmlessly retried.
/// - 500 — transient (DB); the dedup ledger + processing share one
///   transaction, so a rolled-back attempt is retried cleanly.
async fn paddle_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<PaddleWebhookAck>, (StatusCode, &'static str)> {
    use paddle::signature::SignatureError;

    let Some(secret) = state.paddle_webhook_secret.as_deref() else {
        error!("PADDLE_WEBHOOK_SECRET not set; refusing delivery so Paddle retries");
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "webhook secret not configured",
        ));
    };

    let signature = headers
        .get("paddle-signature")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if signature.is_empty() || body.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "missing signature or body"));
    }

    // Verified against the exact bytes received — never a re-serialization.
    match paddle::signature::verify_signature(signature, &body, secret, Utc::now()) {
        Ok(()) => {}
        Err(SignatureError::MalformedHeader | SignatureError::BadTimestamp) => {
            return Err((StatusCode::BAD_REQUEST, "malformed signature header"));
        }
        Err(e @ (SignatureError::StaleTimestamp | SignatureError::Mismatch)) => {
            warn!(error = %e, "paddle signature verification failed");
            return Err((StatusCode::UNAUTHORIZED, "invalid signature"));
        }
    }

    let event: paddle::events::PaddleEvent = match serde_json::from_slice(&body) {
        Ok(event) => event,
        Err(e) => {
            warn!(error = %e, "paddle webhook body is not a parseable event");
            return Err((StatusCode::BAD_REQUEST, "unparseable event"));
        }
    };

    let mut tx = state.pg_pool.begin().await.map_err(paddle_db_error)?;
    let is_new = paddle::sync::record_event(
        &mut tx,
        &event.event_id,
        &event.event_type,
        event.occurred_at,
    )
    .await
    .map_err(paddle_db_error)?;

    if is_new {
        paddle::process_event(&mut tx, &event, state.paddle_api.as_ref())
            .await
            .map_err(paddle_db_error)?;
    }
    tx.commit().await.map_err(paddle_db_error)?;

    info!(
        event_type = %event.event_type,
        event_id = %event.event_id,
        duplicate = !is_new,
        "paddle webhook accepted"
    );
    metrics::counter!(
        "heraldq_paddle_webhooks_total",
        "outcome" => if is_new { "accepted" } else { "duplicate" }
    )
    .increment(1);

    Ok(Json(PaddleWebhookAck {
        received: true,
        duplicate: !is_new,
    }))
}

fn paddle_db_error(e: sqlx::Error) -> (StatusCode, &'static str) {
    error!(error = ?e, "paddle webhook database error");
    (StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

#[derive(Serialize)]
struct EnqueueResponse {
    id: String,
}

async fn enqueue_webhook(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
    extract::Json(request_body): extract::Json<WebhookRequest>,
) -> Result<(StatusCode, Json<EnqueueResponse>), AppError> {
    info!(tenant_id = %api_key.tenant_id, "Webhook enqueue requested");

    if state.webhook_limiter.check_key(&api_key.tenant_id).is_err() {
        return Err(AppError::TooManyRequests);
    };

    validate_destination(&state.policy, &request_body.destination)?;

    // Hard monthly cap, atomically counted in the DB. Only valid requests that
    // will actually enqueue should burn a quota slot.
    if !crate::db::quotas::increment_monthly_quota(
        &state.pg_pool,
        api_key.tenant_id,
        MONTHLY_WEBHOOK_LIMIT,
    )
    .await?
    {
        return Err(AppError::MonthlyQuotaExceeded);
    }

    let webhook_id = webhook::enqueue(&state.worker_utils, api_key.tenant_id, request_body).await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(EnqueueResponse { id: webhook_id }),
    ))
}

/// Signup creates a free-tier tenant and returns credentials once.
///
/// Signup is the front door: it is open (no invite code required), keys do
/// not expire, and any Paddle customer already mirroring this email is
/// linked and its tier applied immediately — so billing state is consistent
/// the moment the account exists, not at Paddle's next webhook.
#[derive(Deserialize)]
struct SignupRequest {
    /// Optional invite code. Absent codes are fine; a present-but-invalid
    /// code is rejected so a typo'd code never silently loses its perk.
    invite_code: Option<String>,
    name: String,
    email: String,
}

#[derive(Serialize)]
struct SignupResponse {
    tenant_id: uuid::Uuid,
    api_key: String,
    hmac_secret: String,
}

/// Deliberately loose: exactly one `@`, non-empty local part, a dot in the
/// domain, no whitespace, sane lengths. Signup's job is to keep junk out of
/// the Paddle email-linkage — not to prove the inbox exists.
fn is_valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && local.len() <= 64
        && domain.len() <= 251
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains('@')
        && !email.chars().any(char::is_whitespace)
}

async fn signup(
    State(state): State<AppState>,
    extract::Json(body): extract::Json<SignupRequest>,
) -> Result<(StatusCode, Json<SignupResponse>), AppError> {
    if state.signup_limiter.check().is_err() {
        return Err(AppError::TooManyRequests);
    };

    // Normalize before anything touches the database: emails are stored
    // lowercased on both sides of the Paddle linkage (signup and the
    // customer mirror), so "You@Example.com" at signup still links to the
    // "you@example.com" Paddle collected at checkout. Mobile keyboards
    // autocapitalize; the linkage must not care.
    let email = body.email.trim().to_lowercase();
    let name = body.name.trim().to_owned();
    let invite_code = body
        .invite_code
        .as_deref()
        .map(str::trim)
        .filter(|code| !code.is_empty());

    if name.is_empty() {
        return Err(AppError::NameRequired);
    }
    if !is_valid_email(&email) {
        return Err(AppError::InvalidEmail);
    }

    let hmac_secret = auth::generate_hmac_secret();

    let mut tx = state.pg_pool.begin().await?;

    let tenant_id = crate::db::tenants::create_tenant(&mut tx, &name, &email, &hmac_secret).await?;

    if let Some(invite_code) = invite_code {
        crate::db::tenants::redeem_invite_code(&mut tx, invite_code, tenant_id).await?;
    }

    // Keys don't expire: the product boundary is tier/quota enforcement,
    // not a time-based trial. A key that lapses mid-subscription would lock
    // a paying customer out.
    let raw_key = auth::generate_api_key();
    let key_hash = auth::hash_api_key(&raw_key);

    crate::db::tenants::create_api_key(&mut tx, tenant_id, &key_hash, "signup", None).await?;

    paddle::sync::link_tenant_and_apply_tier(&mut tx, tenant_id, &body.email).await?;

    tx.commit().await?;

    info!(tenant_id = %tenant_id, "New tenant signed up");

    Ok((
        StatusCode::CREATED,
        Json(SignupResponse {
            tenant_id,
            api_key: raw_key,
            hmac_secret,
        }),
    ))
}

#[derive(Serialize)]
struct KeyResponse {
    api_key: String,
}

/// Shared machinery for rotate and recover: revoke every active key, issue
/// exactly one replacement, return its raw value once. Runs in a single
/// transaction so there is never a moment with two live keys (rotation) or
/// with a stolen key still valid (recovery).
async fn replace_tenant_key(
    state: &AppState,
    tenant_id: uuid::Uuid,
    name: &str,
) -> Result<String, AppError> {
    let raw_key = auth::generate_api_key();
    let key_hash = auth::hash_api_key(&raw_key);

    let mut tx = state.pg_pool.begin().await?;
    crate::db::tenants::revoke_all_keys(&mut tx, tenant_id).await?;
    crate::db::tenants::create_api_key(&mut tx, tenant_id, &key_hash, name, None).await?;
    tx.commit().await?;

    Ok(raw_key)
}

/// `POST /v1/keys/rotate` — swap the caller's key for a fresh one. The old
/// key dies with the request, so rotation is also the "I think this leaked"
/// panic button.
async fn rotate_key(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
) -> Result<(StatusCode, Json<KeyResponse>), AppError> {
    let raw_key = replace_tenant_key(&state, api_key.tenant_id, "rotated").await?;

    info!(tenant_id = %api_key.tenant_id, "API key rotated");
    Ok((StatusCode::OK, Json(KeyResponse { api_key: raw_key })))
}

#[derive(Deserialize)]
struct RecoverRequest {
    email: String,
    hmac_secret: String,
}

/// `POST /v1/keys/recover` — lost-key recovery without email
/// infrastructure. The account email plus the HMAC secret (shown next to
/// the key at signup, and something the tenant functionally must keep to
/// verify our webhook signatures) is the proof of account. Recovering
/// revokes every existing key: whoever recovers, wins — and a surprised
/// legitimate tenant whose key suddenly stopped working has just learned
/// their secret leaked.
///
/// Unknown email and wrong secret answer identically (401, same body), so
/// recovery can't be used to enumerate accounts.
async fn recover_key(
    State(state): State<AppState>,
    extract::Json(body): extract::Json<RecoverRequest>,
) -> Result<(StatusCode, Json<KeyResponse>), AppError> {
    if state.signup_limiter.check().is_err() {
        return Err(AppError::TooManyRequests);
    };

    let email = body.email.trim().to_lowercase();
    let tenant_id = match crate::db::tenants::find_tenant_for_recovery(
        &state.pg_pool,
        &email,
        &body.hmac_secret,
    )
    .await?
    {
        Some(id) => id,
        None => {
            warn!("key recovery failed: email/secret mismatch");
            return Err(AppError::Unauthorized);
        }
    };

    let raw_key = replace_tenant_key(&state, tenant_id, "recovered").await?;

    info!(tenant_id = %tenant_id, "API key recovered");
    Ok((StatusCode::OK, Json(KeyResponse { api_key: raw_key })))
}

#[derive(Serialize)]
struct ApiKeyInfoResponse {
    id: uuid::Uuid,
    name: String,
    created_at: chrono::DateTime<Utc>,
    expires_at: Option<chrono::DateTime<Utc>>,
    is_active: bool,
}

/// `GET /v1/keys` — key metadata for the caller's tenant. Never includes
/// hashes or raw values; those exist only at issue time.
async fn list_keys(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
) -> Result<Json<Vec<ApiKeyInfoResponse>>, AppError> {
    let keys = crate::db::tenants::list_keys(&state.pg_pool, api_key.tenant_id).await?;

    Ok(Json(
        keys.into_iter()
            .map(|k| ApiKeyInfoResponse {
                id: k.id,
                name: k.name,
                created_at: k.created_at,
                expires_at: k.expires_at,
                is_active: k.is_active,
            })
            .collect(),
    ))
}

async fn webhook_activity_summary(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
) -> Result<Json<WebhookActivitySummary>, AppError> {
    let summary = crate::db::stats::activity_summary(&state.pg_pool, api_key.tenant_id).await?;

    Ok(Json(summary))
}

async fn dashboard(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
) -> Result<Html<String>, AppError> {
    let counts = crate::db::stats::activity_summary(&state.pg_pool, api_key.tenant_id).await?;

    Ok(Html(crate::views::render_dashboard(&counts)))
}

#[derive(Serialize)]
struct DeadLetterResponse {
    messages: Vec<crate::db::dlq::DlqEntry>,
    dlq_total: i64,
    limit: i64,
    offset: i64,
}

#[derive(Deserialize)]
struct Pagination {
    limit: Option<i64>,
    offset: Option<i64>,
}

fn clamp_pagination(value: Option<i64>, default: i64, max: i64) -> i64 {
    value.unwrap_or(default).clamp(0, max)
}

async fn dlq(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
    Query(pagination): Query<Pagination>,
) -> Result<Json<DeadLetterResponse>, AppError> {
    let tenant_id = api_key.tenant_id;

    let limit = clamp_pagination(pagination.limit, 25, 100);
    let offset = clamp_pagination(pagination.offset, 0, i64::MAX);

    let messages =
        crate::db::dlq::list_page(&state.pg_pool, &api_key.tenant_id, limit, offset).await?;
    let dlq_total = crate::db::dlq::count_dlq_total(&state.pg_pool, tenant_id).await?;

    Ok(Json(DeadLetterResponse {
        messages,
        dlq_total,
        limit,
        offset,
    }))
}

#[derive(Serialize)]
struct DeliveriesResponse {
    deliveries: Vec<crate::db::deliveries::DeliveryRecord>,
    total: i64,
    limit: i64,
    offset: i64,
}

async fn deliveries(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
    Query(pagination): Query<Pagination>,
) -> Result<Json<DeliveriesResponse>, AppError> {
    let tenant_id = api_key.tenant_id;

    let limit = clamp_pagination(pagination.limit, 25, 100);
    let offset = clamp_pagination(pagination.offset, 0, i64::MAX);

    let (deliveries, total) = tokio::try_join!(
        crate::db::deliveries::list_page(&state.pg_pool, tenant_id, limit, offset),
        crate::db::deliveries::count_total(&state.pg_pool, tenant_id),
    )?;

    Ok(Json(DeliveriesResponse {
        deliveries,
        total,
        limit,
        offset,
    }))
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum WebhookStatusResponse {
    Delivered {
        id: String,
        destination: String,
        status_code: Option<i16>,
        attempts: Option<i16>,
        enqueued_at: chrono::DateTime<Utc>,
        delivered_at: chrono::DateTime<Utc>,
    },
    Failed {
        id: String,
        destination: String,
        failure_reason: Option<String>,
        status_code: Option<i16>,
        attempts: Option<i16>,
        enqueued_at: chrono::DateTime<Utc>,
        abandoned_at: chrono::DateTime<Utc>,
    },
    Pending {
        id: String,
        destination: String,
        attempts: i16,
        max_attempts: i16,
        last_error: Option<String>,
        enqueued_at: chrono::DateTime<Utc>,
        next_attempt_at: chrono::DateTime<Utc>,
    },
}

impl From<crate::db::deliveries::DeliveryRecord> for WebhookStatusResponse {
    fn from(d: crate::db::deliveries::DeliveryRecord) -> Self {
        Self::Delivered {
            id: d.id,
            destination: d.destination,
            status_code: d.status_code,
            attempts: d.attempts,
            enqueued_at: d.enqueued_at,
            delivered_at: d.delivered_at,
        }
    }
}

impl From<crate::db::dlq::DlqEntry> for WebhookStatusResponse {
    fn from(f: crate::db::dlq::DlqEntry) -> Self {
        Self::Failed {
            id: f.id,
            destination: f.destination,
            failure_reason: f.failure_reason,
            status_code: f.status_code,
            attempts: f.attempts,
            enqueued_at: f.enqueued_at,
            abandoned_at: f.abandoned_at,
        }
    }
}

#[derive(Serialize)]
struct ReplayResponse {
    /// The new delivery attempt's webhook id — poll /webhook/{id} with it.
    id: String,
    replayed_from: String,
}

/// Re-enqueue a dead-lettered webhook as a fresh delivery attempt.
///
/// The DLQ row is claimed atomically before the job is created, so a
/// double-click can't produce two deliveries: the second request gets a 409
/// carrying the id the first one minted. The replay gets a NEW webhook id —
/// reusing the original would collide with graphile's job-key row for the
/// exhausted job, and the DLQ row (kept, stamped replayed_at/-as) still
/// records the original.
async fn dlq_replay(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
    Path(webhook_id): Path<String>,
) -> Result<(StatusCode, Json<ReplayResponse>), AppError> {
    let tenant_id = api_key.tenant_id;

    // Same per-tenant throttle as fresh enqueues: a replay is an outbound
    // webhook like any other.
    if state.webhook_limiter.check_key(&tenant_id).is_err() {
        return Err(AppError::TooManyRequests);
    };

    let new_id = ulid::Ulid::new().to_string();
    let Some(source) =
        crate::db::dlq::claim_for_replay(&state.pg_pool, tenant_id, &webhook_id, &new_id).await?
    else {
        // Claimed by nobody = not this tenant's row; claimed already = 409
        // with the existing replay id so the caller can follow it.
        return match crate::db::dlq::find_by_id(&state.pg_pool, tenant_id, &webhook_id).await? {
            Some(entry) => Err(AppError::AlreadyReplayed {
                replayed_as: entry.replayed_as.unwrap_or_default(),
            }),
            None => Err(AppError::NotFound),
        };
    };

    // The destination passed SSRF policy at original enqueue time, but
    // policy can change between then and a replay months later — re-check,
    // and release the claim on any failure so the tenant can retry.
    let replay = async {
        validate_destination(&state.policy, &source.destination)?;
        webhook::enqueue_with_id(
            &state.worker_utils,
            new_id.clone(),
            tenant_id,
            WebhookRequest {
                destination: source.destination,
                payload: source.payload,
            },
        )
        .await?;
        Ok::<_, AppError>(())
    }
    .await;

    if let Err(e) = replay {
        crate::db::dlq::unclaim_replay(&state.pg_pool, tenant_id, &webhook_id, &new_id).await?;
        return Err(e);
    }

    info!(tenant_id = %tenant_id, replayed_from = %webhook_id, id = %new_id, "DLQ entry replayed");

    Ok((
        StatusCode::ACCEPTED,
        Json(ReplayResponse {
            id: new_id,
            replayed_from: webhook_id,
        }),
    ))
}

async fn webhook_status(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
    Path(webhook_id): Path<String>,
) -> Result<Json<WebhookStatusResponse>, AppError> {
    let pool = &state.pg_pool;
    let tenant_id = api_key.tenant_id;

    // Terminal states first — once written they never change.
    if let Some(d) = crate::db::deliveries::find_by_id(pool, tenant_id, &webhook_id).await? {
        return Ok(Json(d.into()));
    }
    if let Some(f) = crate::db::dlq::find_by_id(pool, tenant_id, &webhook_id).await? {
        return Ok(Json(f.into()));
    }
    if let Some(p) = crate::db::jobs::find_by_webhook_id(pool, tenant_id, &webhook_id).await? {
        return Ok(Json(WebhookStatusResponse::Pending {
            id: webhook_id,
            destination: p.destination,
            attempts: p.attempts,
            max_attempts: p.max_attempts,
            last_error: p.last_error,
            enqueued_at: p.created_at,
            next_attempt_at: p.run_at,
        }));
    }

    // The job may have reached a terminal state between the checks above and
    // the queue lookup (the terminal row is committed before the job row is
    // deleted) — re-check the terminal stores once before declaring 404.
    if let Some(d) = crate::db::deliveries::find_by_id(pool, tenant_id, &webhook_id).await? {
        return Ok(Json(d.into()));
    }
    if let Some(f) = crate::db::dlq::find_by_id(pool, tenant_id, &webhook_id).await? {
        return Ok(Json(f.into()));
    }

    Err(AppError::NotFound)
}

async fn mock(State(_state): State<AppState>) -> StatusCode {
    let roll = rand::rng().random_range(0..100);

    let status = if roll < 5 {
        StatusCode::TOO_MANY_REQUESTS
    } else if roll < 10 {
        StatusCode::IM_A_TEAPOT
    } else if roll < 20 {
        StatusCode::REQUEST_TIMEOUT
    } else if roll < 30 {
        StatusCode::TOO_EARLY
    } else if roll < 40 {
        StatusCode::PERMANENT_REDIRECT
    } else if roll < 85 {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::OK
    };

    debug!(roll, status = status.as_u16(), "Mock endpoint hit");
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    // The `status` tag and snake_case variant names are the public API
    // contract for GET /webhook/{id} — a rename here breaks customers.
    #[test]
    fn webhook_status_serializes_with_status_tag() {
        let now = Utc::now();
        let pending = WebhookStatusResponse::Pending {
            id: "01TEST".into(),
            destination: "https://example.com".into(),
            attempts: 3,
            max_attempts: 15,
            last_error: Some("500 Internal Server Error".into()),
            enqueued_at: now,
            next_attempt_at: now,
        };
        let json = serde_json::to_value(&pending).unwrap();
        assert_eq!(json["status"], "pending");
        assert_eq!(json["attempts"], 3);
        assert_eq!(json["last_error"], "500 Internal Server Error");
    }

    #[test]
    fn email_validator_accepts_real_shapes() {
        for good in [
            "you@example.com",
            "first.last+tag@sub.example.co",
            "a@b.io",
            "USER@EXAMPLE.COM",
        ] {
            assert!(is_valid_email(good), "should accept {good}");
        }
    }

    #[test]
    fn email_validator_rejects_junk() {
        for bad in [
            "",
            "no-at-sign",
            "@example.com",
            "user@",
            "user@example",
            "user @example.com",
            "user@exa mple.com",
            "a@b@c.com",
            "user@.example.com",
            "user@example.com.",
        ] {
            assert!(!is_valid_email(bad), "should reject {bad:?}");
        }
    }
}
