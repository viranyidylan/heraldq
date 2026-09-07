//! Account creation. One route: the open front door.

use axum::{Json, extract};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{AppState, auth, error::AppError, paddle};

/// Signup creates a free-tier tenant and returns credentials once.
///
/// Signup is the front door: it is open, keys do not expire, and any
/// Paddle customer already mirroring this email is
/// linked and its tier applied immediately — so billing state is consistent
/// the moment the account exists, not at Paddle's next webhook.
#[derive(Deserialize)]
pub(super) struct SignupRequest {
    name: String,
    email: String,
}

#[derive(Serialize)]
pub(super) struct SignupResponse {
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

pub(super) async fn signup(
    extract::State(state): extract::State<AppState>,
    extract::Json(body): extract::Json<SignupRequest>,
) -> Result<(StatusCode, Json<SignupResponse>), AppError> {
    // Signup funnel: one counter, one outcome label per exit. Duplicate is
    // expected noise (double-clicks, retries); invalid and rate_limited are
    // the signals worth watching as the open front door.
    if state.signup_limiter.check().is_err() {
        metrics::counter!("heraldq_signups_total", "outcome" => "rate_limited").increment(1);
        return Err(AppError::TooManyRequests);
    };

    // Normalize before anything touches the database: emails are stored
    // lowercased on both sides of the Paddle linkage (signup and the
    // customer mirror), so "You@Example.com" at signup still links to the
    // "you@example.com" Paddle collected at checkout. Mobile keyboards
    // autocapitalize; the linkage must not care.
    let email = body.email.trim().to_lowercase();
    let name = body.name.trim().to_owned();

    if name.is_empty() {
        metrics::counter!("heraldq_signups_total", "outcome" => "invalid").increment(1);
        return Err(AppError::NameRequired);
    }
    if !is_valid_email(&email) {
        metrics::counter!("heraldq_signups_total", "outcome" => "invalid").increment(1);
        return Err(AppError::InvalidEmail);
    }

    let hmac_secret = auth::generate_hmac_secret();

    let mut tx = state.pg_pool.begin().await?;

    let tenant_id =
        match crate::db::tenants::create_tenant(&mut tx, &name, &email, &hmac_secret).await {
            Ok(id) => id,
            Err(e @ AppError::EmailAlreadyRegistered) => {
                metrics::counter!("heraldq_signups_total", "outcome" => "duplicate").increment(1);
                return Err(e);
            }
            Err(e) => return Err(e),
        };

    // Keys don't expire: the product boundary is tier/quota enforcement,
    // not a time-based trial. A key that lapses mid-subscription would lock
    // a paying customer out.
    let raw_key = auth::generate_api_key();
    let key_hash = auth::hash_api_key(&raw_key);

    crate::db::tenants::create_api_key(&mut tx, tenant_id, &key_hash, "signup", None).await?;

    // `&email`, never `&body.email`: the linkage matches paddle_customers on
    // the lowercased form, so passing the raw input here would silently miss
    // the link for any mixed-case signup.
    paddle::sync::link_tenant_and_apply_tier(&mut tx, tenant_id, &email).await?;

    tx.commit().await?;

    info!(tenant_id = %tenant_id, "New tenant signed up");
    metrics::counter!("heraldq_signups_total", "outcome" => "created").increment(1);

    Ok((
        StatusCode::CREATED,
        Json(SignupResponse {
            tenant_id,
            api_key: raw_key,
            hmac_secret,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
