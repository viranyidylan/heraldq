//! API key lifecycle: list, rotate, and lost-key recovery.
//!
//! `recover_key` is registered in `super::build` rather than in `routes()`
//! below, because it is one of the two browser-facing routes and has to sit
//! inside the CORS `route_layer`.

use axum::{
    Json, Router,
    extract::{self, State},
    routing::{get, post},
};
use chrono::Utc;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::{AppState, auth, error::AppError};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/keys", get(list_keys))
        .route("/keys/rotate", post(rotate_key))
}

#[derive(Serialize)]
pub(super) struct KeyResponse {
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
pub(super) async fn rotate_key(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
) -> Result<(StatusCode, Json<KeyResponse>), AppError> {
    let raw_key = replace_tenant_key(&state, api_key.tenant_id, "rotated").await?;

    info!(tenant_id = %api_key.tenant_id, "API key rotated");
    Ok((StatusCode::OK, Json(KeyResponse { api_key: raw_key })))
}

#[derive(Deserialize)]
pub(super) struct RecoverRequest {
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
pub(super) async fn recover_key(
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
pub(super) struct ApiKeyInfoResponse {
    id: uuid::Uuid,
    name: String,
    created_at: chrono::DateTime<Utc>,
    expires_at: Option<chrono::DateTime<Utc>>,
    is_active: bool,
}

/// `GET /v1/keys` — key metadata for the caller's tenant. Never includes
/// hashes or raw values; those exist only at issue time.
pub(super) async fn list_keys(
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
