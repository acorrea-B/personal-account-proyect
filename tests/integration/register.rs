//! The register flow E2E (T011, US1, FR-001..003): the 201 WITHOUT
//! credentials, the normalization of the email (FR-002) in the response AND
//! in the DB, the 409 `email-exists` case-insensitive, and the 422s of the
//! catalogue (`email-malformed`, `weak-password` with the R4 requirements).
//!
//! The already-covered cousins live in their owns (no duplicates): the
//! 403 `email-not-verified` with `can_resend` in `session_flow` (T014) and
//! the generic resend + the mirror of the confirmation in `profile_mirror`
//! (T013, FR-004/FR-006).

use crate::common::{app, call, db_pool, json_of};

use axum::http::StatusCode;
use sqlx::Row;
use uuid::Uuid;

/// FR-001..002 E2E: the register answers 201 with ONLY the user (FR-003:
/// no tokens — the session is a separate contract) and the normalized
/// email (trim + lowercase) travels in the response AND in the DB.
#[tokio::test]
async fn the_register_answers_201_without_credentials() {
    let Some(pool) = db_pool().await else {
        return;
    };
    let Some(app) = app().await else {
        return;
    };

    let email = format!("Register-{}@test.local", Uuid::new_v4().simple());
    let response = call(
        &app,
        "POST",
        "/v1/auth/register",
        Some(serde_json::json!({
            "email": format!("  {email} "), // the case and the spaces travel (FR-002)
            "password": "contraseña1"
        })),
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "FR-001: the register accepts a valid account"
    );
    let body = json_of(response).await;

    // FR-003: the 201 NEVER travels with credentials — only the user, and
    // the user with only its public face.
    assert!(
        body.get("access_token").is_none() && body.get("refresh_token").is_none(),
        "FR-003: no tokens in the 201"
    );
    let user = body["user"].as_object().expect("the 201 carries the user");
    assert_eq!(
        user.keys().collect::<Vec<_>>().len(),
        4,
        "the user's public face only: id, email, verified, created_at"
    );
    assert_eq!(
        body["user"]["email"],
        email.to_lowercase(),
        "FR-002: the email normalized in the response"
    );
    assert!(body["user"]["verified"].is_boolean(), "the honest flavor");
    assert!(
        body["user"]["created_at"].is_string(),
        "the public face carries the instant"
    );

    // FR-002 in the DB: the email landed in `auth.users` normalized too.
    let id: Uuid = body["user"]["id"]
        .as_str()
        .expect("the 201 carries the id")
        .parse()
        .expect("a UUID");
    let db_email: String = sqlx::query("select email from auth.users where id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("the fixture's row exists")
        .get("email");
    assert_eq!(
        db_email,
        email.to_lowercase(),
        "FR-002: the email normalized in the DB"
    );
}

/// FR-002 E2E: the SAME email in any case (and with its spaces) never
/// clones an account — the normalized address collides with the first one
/// in whichever dialect goTrue speaks: `email-exists` (409) of the
/// catalogue when the stack rejects the repetition, or `rate-limited` (429)
/// when the honest-verification stack folds the repetition into the
/// confirmation resend and the 1h email window denies it (goTrue's
/// anti-enumeration: an INEXISTENT repeated address hits the same window —
/// the answer leaks no existence). BOTH = one row in the DB.
#[tokio::test]
async fn the_same_email_never_clones_the_account_in_any_case() {
    let Some(pool) = db_pool().await else {
        return;
    };
    let Some(app) = app().await else {
        return;
    };

    let email = format!("Duplicate-{}@test.local", Uuid::new_v4().simple());
    let first = call(
        &app,
        "POST",
        "/v1/auth/register",
        Some(serde_json::json!({ "email": email, "password": "contraseña1" })),
    )
    .await;
    assert_eq!(first.status(), StatusCode::CREATED, "the first one wins");

    // The SAME address, uppercased and surrounded by spaces: a problem of
    // the catalogue (409 or 429), never a 201 of a clone account.
    let second = call(
        &app,
        "POST",
        "/v1/auth/register",
        Some(serde_json::json!({
            "email": format!("  {} ", email.to_uppercase()),
            "password": "contraseña1"
        })),
    )
    .await;
    let second_status = second.status();
    let body = json_of(second).await;
    let slug = body["type"]
        .as_str()
        .and_then(|t| t.rsplit('/').next())
        .unwrap_or("no-slug");
    match second_status {
        StatusCode::CONFLICT => assert_eq!(
            slug, "email-exists",
            "the catalog of the repetition (the rejecting stack)"
        ),
        StatusCode::TOO_MANY_REQUESTS => assert_eq!(
            slug, "rate-limited",
            "the honest-verification flavor: the repetition IS a resend, the 1h window denies it"
        ),
        other => {
            panic!("FR-002: the repetition never answers {other}; the catalogue or the window")
        }
    }

    // THE invariant of the collision: exactly ONE row holds the address —
    // the uppercased/spaced variant never lands as a clone.
    let rows = sqlx::query_scalar::<_, i64>(
        "select count(*) from auth.users where lower(email) = lower($1)",
    )
    .bind(&email)
    .fetch_one(&pool)
    .await
    .expect("the collision count answers");
    assert_eq!(rows, 1, "FR-002: the repetition adds no row");
}

/// The catalogue E2E: the malformed email answers the 422 `email-malformed`
/// (the validation runs on the NORMALIZED email — the spaces never make it
/// malformed before its turn).
#[tokio::test]
async fn the_malformed_email_answers_422() {
    let Some(app) = app().await else {
        return;
    };

    let response = call(
        &app,
        "POST",
        "/v1/auth/register",
        Some(serde_json::json!({ "email": "no-es-un-email", "password": "contraseña1" })),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "the malformed email never reaches Supabase"
    );
    let body = json_of(response).await;
    assert!(
        body["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("email-malformed")),
        "the slug of the catalogue answers"
    );
}

/// R4 E2E: the weak password answers the 422 `weak-password` with the
/// published requirements of the catalogue as its extension — the client
/// reads WHAT is missing, never a mystery.
#[tokio::test]
async fn the_weak_password_answers_422_with_the_requirements() {
    let Some(app) = app().await else {
        return;
    };

    let email = format!("Weak-{}@test.local", Uuid::new_v4().simple());
    let response = call(
        &app,
        "POST",
        "/v1/auth/register",
        Some(serde_json::json!({ "email": email, "password": "short" })),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "R4: the password below the policy answers 422"
    );
    let body = json_of(response).await;
    assert!(
        body["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("weak-password")),
        "the slug of the catalogue answers"
    );
    assert_eq!(
        body["requirements"],
        serde_json::json!(["length ≥ 8", "≥ 1 letter", "≥ 1 number"]),
        "the published requirements of R4 travel in the extension"
    );
}
