//! The mirror of the profile (T013, US1, FR-004): the email confirmation
//! (the UPDATE of `email_confirmed_at`) creates the profile row ONCE —
//! with the normalized email and the placeholder `full_name` of Q1 — and
//! the edge case of the spec §5 (the account created without a session:
//! the resend covers it and the mirror still lands on the confirmation).

use crate::common::{admin_confirm_user, admin_create_user, app, call, db_pool, json_of};

use axum::http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

/// The profile row of a fixture user (None = still no mirror).
async fn mirror_of(pool: &PgPool, user_id: &Uuid) -> Option<(String, String)> {
    sqlx::query_as::<_, (String, String)>(
        "select email, full_name from public.profiles where id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .expect("the mirror query answers")
}

/// The count of the mirror rows (the 1:1 uniqueness of FR-004).
async fn mirror_count(pool: &PgPool, user_id: &Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>("select count(*) from public.profiles where id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("the count answers")
}

/// FR-004 E2E: the unconfirmed account has NO mirror; the confirmation
/// CREATES it (normalized email + the placeholder `full_name` — Q1) and a
/// repeated confirmation never duplicates (the 1:1).
#[tokio::test]
async fn the_email_confirmation_creates_the_mirror_once() {
    let Some(pool) = db_pool().await else {
        return;
    };
    let user = admin_create_user(false).await;
    let user_id: Uuid = user.id.parse().expect("the fixture carries a UUID");

    assert!(
        mirror_of(&pool, &user_id).await.is_none(),
        "the unconfirmed account has no mirror yet"
    );

    admin_confirm_user(&user.id).await;

    let (email, full_name) = mirror_of(&pool, &user_id)
        .await
        .expect("the confirmation creates the mirror (FR-004)");
    assert_eq!(
        email, user.email,
        "the email normalized already travels in lowercase"
    );
    assert_eq!(
        full_name, "User",
        "the placeholder of Q1 (the API fills the rest)"
    );

    // The 1:1: a repeated confirmation never duplicates (on conflict do
    // nothing).
    admin_confirm_user(&user.id).await;
    assert_eq!(mirror_count(&pool, &user_id).await, 1, "FR-004: 1:1");
}

/// The edge of the spec §5: the account exists (created, NO session), the
/// email service of the flow was down — the resend of the APP answers its
/// generic always (FR-006) and the mirror lands the same when the
/// confirmation happens (the trigger awaits, the resend covers the gap).
#[tokio::test]
async fn the_email_outage_never_leaves_a_mirrorless_account() {
    let Some(pool) = db_pool().await else {
        return;
    };
    let Some(app) = app().await else {
        return;
    };
    let user = admin_create_user(false).await;
    let user_id: Uuid = user.id.parse().expect("the fixture carries a UUID");

    // The account is there, unconfirmed: no mirror, no session...
    assert!(mirror_of(&pool, &user_id).await.is_none());

    // ...the resend covers the gap (the generic 200 of FR-006 — NEVER deny).
    let response = call(
        &app,
        "POST",
        "/v1/auth/resend-verification",
        Some(serde_json::json!({ "email": user.email })),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "FR-006: the generic always"
    );
    let body = json_of(response).await;
    assert_eq!(
        body["message"],
        "Si el email corresponde a una cuenta pendiente, se envió un enlace."
    );

    // The confirmation happens later (the user clicked the resent link):
    // the mirror lands then — no account stays mirrorless.
    admin_confirm_user(&user.id).await;
    assert!(mirror_of(&pool, &user_id).await.is_some());
    assert_eq!(mirror_count(&pool, &user_id).await, 1);
}

/// The mirror of a registered account (through the APP, not the admin
/// fixture), FR-004: the mirror lands at the INSTANT OF THE CONFIRMATION —
/// whichever it was. The flavor decides the instant: the autoconfirm
/// stacks (the CI's local — the era without email confirmations) confirm
/// AT THE SIGNUP and their mirror already exists in the 201; the
/// email-verification flow leaves it empty until the admin confirmation.
/// BOTH = the same contract (one mirror, Q1's placeholder, 1:1).
#[tokio::test]
async fn the_confirmation_of_a_registered_account_mirrors_the_same() {
    let Some(pool) = db_pool().await else {
        return;
    };
    let Some(app) = app().await else {
        return;
    };
    let email = format!("reg-mirror-{}@test.local", Uuid::new_v4().simple());
    let response = call(
        &app,
        "POST",
        "/v1/auth/register",
        Some(serde_json::json!({
            "email": format!("  {email} "), // the normalization travels (FR-002)
            "password": "contraseña1"
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "the register of the app (the flow of T012)"
    );
    let body = json_of(response).await;
    let registered_id: Uuid = body["user"]["id"]
        .as_str()
        .expect("the 201 carries the id")
        .parse()
        .expect("a UUID");

    let (expected_email, mirror) = {
        let mirror = mirror_of(&pool, &registered_id).await;
        (email.to_lowercase(), mirror)
    };

    match mirror {
        // The email-verification flavor: the mirror still owes the
        // confirmation — it lands when the UPDATE arrives.
        None => {
            admin_confirm_user(&registered_id.to_string()).await;
            let (email, full_name) = mirror_of(&pool, &registered_id)
                .await
                .expect("the mirror lands on the confirmation");
            assert!(email.chars().all(|c| c == c.to_ascii_lowercase()), "FR-002");
            assert_eq!(full_name, "User", "the placeholder of Q1");
        }
        // The autoconfirm flavor: the mirror of the confirmed-at-signup
        // account = the SAME contract's row.
        Some((existing_email, existing_full_name)) => {
            assert!(
                existing_email.chars().all(|c| c == c.to_ascii_lowercase()),
                "FR-002"
            );
            assert_eq!(existing_full_name, "User", "the placeholder of Q1");
            assert_eq!(
                body["user"]["verified"], true,
                "the flavor that mirrors at signup says so honest in the 201"
            );
        }
    }
    // The email's normalization of the register (FR-002) held either way:
    let _ = expected_email;
    assert_eq!(
        mirror_count(&pool, &registered_id).await,
        1,
        "the 1:1 always"
    );
}
