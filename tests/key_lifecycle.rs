//! Route-level tests for the API key lifecycle: rotate, list, and
//! lost-key recovery. These drive the real router (auth extractor, JSON
//! bodies, status codes) with `oneshot`, against a throwaway Postgres.
//!
//! ```sh
//! export $(grep DATABASE_URL .env) && cargo test -- --ignored
//! ```

use axum::body::Body;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

use sliq::AppState;
use sliq::routes;

/// Build the real production router over a test pool. WorkerUtils is just
/// pool + schema (enqueue would fail without a live worker, but none of
/// these tests enqueue).
fn app(pool: PgPool) -> axum::Router {
    use std::sync::Arc;

    let state = AppState {
        pg_pool: pool.clone(),
        worker_utils: graphile_worker::WorkerUtils::new(pool.clone(), "graphile_worker".into()),
        schema: "graphile_worker".into(),
        signup_limiter: Arc::new(governor::RateLimiter::direct(governor::Quota::per_minute(
            nonzero_ext::nonzero!(100u32),
        ))),
        webhook_limiter: Arc::new(governor::RateLimiter::keyed(governor::Quota::per_second(
            nonzero_ext::nonzero!(10u32),
        ))),
        policy: sliq::webhook::ssrf::policy(),
        paddle_webhook_secret: None,
        paddle_api: None,
        cors_allowed_origins: vec![],
    };
    routes::build(state)
}

async fn signup(pool: &PgPool, email: &str) -> Value {
    let response = app(pool.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/signup")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "name": "Key Test", "email": email }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn call(
    pool: &PgPool,
    method: &str,
    uri: &str,
    key: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(key) = key {
        builder = builder.header("authorization", format!("Bearer {key}"));
    }
    let body = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let response = app(pool.clone())
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

async fn recover(pool: &PgPool, email: &str, secret: &str) -> (StatusCode, Value) {
    call(
        pool,
        "POST",
        "/v1/keys/recover",
        None,
        Some(serde_json::json!({ "email": email, "hmac_secret": secret })),
    )
    .await
}

/// Rotation is atomic from the caller's view: the response carries a new
/// working key, the old one is dead immediately, and exactly one key is
/// active afterwards.
#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn rotate_kills_old_key_and_issues_working_new_one(pool: PgPool) {
    let signup = signup(&pool, "rotate@test.invalid").await;
    let old_key = signup["api_key"].as_str().unwrap().to_owned();

    let (status, body) = call(&pool, "POST", "/v1/keys/rotate", Some(&old_key), None).await;
    assert_eq!(status, StatusCode::OK, "rotate with a valid key");
    let new_key = body["api_key"].as_str().unwrap().to_owned();
    assert_ne!(new_key, old_key);

    // New key authenticates; old key is dead.
    let (status, _) = call(&pool, "GET", "/v1/keys", Some(&new_key), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&pool, "GET", "/v1/keys", Some(&old_key), None).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "rotated-away key must not work"
    );

    // Exactly one active key remains, named for what happened.
    let active: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM api_keys ak
         JOIN tenants t ON t.id = ak.tenant_id
         WHERE t.email = 'rotate@test.invalid' AND ak.is_active = true",
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap();
    assert_eq!(active, 1);
}

/// Recovery requires the email + HMAC secret pair, revokes everything that
/// existed, and answers identically for wrong secrets and unknown emails
/// (no account enumeration).
#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn recover_with_hmac_secret_revokes_and_reissues(pool: PgPool) {
    let signup = signup(&pool, "recover@test.invalid").await;
    let old_key = signup["api_key"].as_str().unwrap().to_owned();
    let secret = signup["hmac_secret"].as_str().unwrap().to_owned();

    // Wrong secret: 401, and the existing key is untouched.
    let (status, _) = recover(&pool, "recover@test.invalid", "wrong-secret").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(&pool, "GET", "/v1/keys", Some(&old_key), None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "failed recovery must not disrupt the key"
    );

    // Unknown email: same status and body shape as wrong secret.
    let (unknown_status, unknown_body) =
        recover(&pool, "nobody@test.invalid", "wrong-secret").await;
    let (wrong_status, wrong_body) = recover(&pool, "recover@test.invalid", "also-wrong").await;
    assert_eq!(unknown_status, wrong_status);
    assert_eq!(unknown_body, wrong_body, "no account enumeration");

    // Correct pair: new key issued, old key revoked immediately.
    let (status, body) = recover(&pool, "recover@test.invalid", &secret).await;
    assert_eq!(status, StatusCode::OK);
    let new_key = body["api_key"].as_str().unwrap().to_owned();

    let (status, _) = call(&pool, "GET", "/v1/keys", Some(&new_key), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&pool, "GET", "/v1/keys", Some(&old_key), None).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "recovery must revoke the lost key"
    );
}

/// The key listing shows metadata only — never a hash, never a raw key —
/// and reflects rotation history.
#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn list_shows_metadata_never_secrets(pool: PgPool) {
    let signup = signup(&pool, "list@test.invalid").await;
    let key = signup["api_key"].as_str().unwrap().to_owned();
    let signup2_secret = signup["hmac_secret"].as_str().unwrap().to_owned();

    let (status, _) = call(&pool, "POST", "/v1/keys/rotate", Some(&key), None).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = call(&pool, "GET", "/v1/keys", Some(&key), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "must use the rotated key");

    // Re-list with the new key by recovering (the old one is gone).
    let (_, recovered) = recover(&pool, "list@test.invalid", &signup2_secret).await;
    let live_key = recovered["api_key"].as_str().unwrap().to_owned();

    let (status, body) = call(&pool, "GET", "/v1/keys", Some(&live_key), None).await;
    assert_eq!(status, StatusCode::OK);

    let rendered = body.to_string();
    assert!(
        !rendered.contains("sliq_"),
        "raw key values must never appear"
    );
    assert!(!rendered.contains("key_hash"), "hashes must never appear");

    let keys = body.as_array().unwrap();
    assert!(keys.len() >= 3, "signup + rotated + recovered all listed");
    assert_eq!(
        keys.iter()
            .filter(|k| k["is_active"].as_bool().unwrap())
            .count(),
        1,
        "exactly one active key"
    );
}
