//! The profile flow (T017, US3): the read and the full-put over the REAL
//! app (the harness — the login of a fixture user with its mirror via the
//! admin confirmation) — the contract's optimistic precondition included.
//!
//! CAPABILITY PROBE (learned.md): the legacy local stack answers an EMPTY
//! JWKS — its credentials verify nothing: the first protected call decides
//! whether the stack can carry the flow (the CI's modern one does; the
//! legacy documents and ends).

use crate::common::{admin_confirm_user, admin_create_user, app, json_of};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use std::time::Duration;

/// One request WITH the credential (the protected flows of this suite):
/// the bearer travels with the correlation id — the same plumbing of
/// `call`, local to the file (the T016's session_flow has its own).
async fn call_with_credentials(
    application: &perfil_api::Application,
    method: &str,
    path: &str,
    bearer: &str,
    body: Option<serde_json::Value>,
) -> axum::response::Response {
    use tower::ServiceExt;
    let mut builder = Request::builder().method(method).uri(path);
    builder = builder
        .header("x-request-id", crate::common::new_request_id())
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    let request = match body {
        Some(value) => builder
            .header("content-type", "application/json")
            .body(Body::from(value.to_string()))
            .expect("the request is valid"),
        None => builder.body(Body::empty()).expect("the request is valid"),
    };
    application
        .router
        .clone()
        .oneshot(request)
        .await
        .expect("the router never fails")
}

/// The session of a user with the mirror landed: (access token, the
/// fixture's email). None = the stack cannot carry the flows (the
/// legacy's empty JWKS) — the caller documents and ends.
///
/// The mirror = ONLY after the admin's CONFIRM (the UPDATE fires the
/// trigger — the INSERT-confirmed does not, in any flavor).
async fn session_with_mirror() -> Option<(perfil_api::Application, crate::common::TestUser, String)>
{
    // The fixture: created unconfirmed, then confirmED — the UPDATE of the
    // trigger (the mirror's guaranteed landing in both flavors).
    let app = app().await?;
    let user = admin_create_user(false).await;
    admin_confirm_user(&user.id).await;

    let login = crate::common::call(
        &app,
        "POST",
        "/v1/auth/login",
        Some(serde_json::json!({ "email": user.email, "password": user.password })),
    )
    .await;
    assert_eq!(
        login.status(),
        StatusCode::OK,
        "the confirmation lets the login in"
    );
    let login = json_of(login).await;
    let access = login["access_token"]
        .as_str()
        .expect("the credential")
        .to_string();

    // The probe (the harness's shared one): the stack's gate decides —
    // the legacy documents and ends, a transient doubts, anything else is
    // loud. The suite never FAILS on the machine's stack quirks; the CI
    // carries the flows.
    use crate::common::Probe;
    match crate::common::capability_probe(|| {
        call_with_credentials(&app, "GET", "/v1/profile", &access, None)
    })
    .await
    {
        Probe::Carry => {
            // The gate verified and served: the suite proceeds.
        }
        Probe::Legacy => {
            eprintln!(
                "the stack verifies nothing through JWKS (legacy): the profile flow pends the CI"
            );
            return None;
        }
        Probe::Undecided => {
            eprintln!("the probe could not decide: the profile flow pends the CI");
            return None;
        }
    }
    Some((app, user, access))
}

/// FR-012/013: the read answers the mirror of the credentials' user ONLY
/// — the placeholder of Q1 + the contract's shape; the internal columns
/// (FR-013, D15-reserved) never travel.
#[tokio::test]
async fn the_profile_read_answers_the_mirror_shape() {
    let Some((app, _user, access)) = session_with_mirror().await else {
        return;
    };

    let response = call_with_credentials(&app, "GET", "/v1/profile", &access, None).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the verified account reads its mirror"
    );
    let body = json_of(response).await;

    let profile = &body["profile"];
    assert_eq!(profile["full_name"], "User", "the placeholder of Q1");
    assert!(
        !profile["email"].as_str().unwrap_or_default().is_empty(),
        "the mirror's email travels"
    );
    assert!(
        profile.get("processing_consent").is_none(),
        "FR-013: the reserved never travels"
    );
    chrono::DateTime::parse_from_rfc3339(profile["updated_at"].as_str().expect("instant"))
        .expect("the updated is ISO8601");
}

/// The FULL put (FR-014): the request's fields save + the omissions EMPTY
/// (the contract's full put, not a patch); the answer reflects and the
/// next GET agrees.
#[tokio::test]
async fn the_full_put_saves_reflects_and_empties_the_omitted() {
    let Some((app, _user, access)) = session_with_mirror().await else {
        return;
    };

    let full = serde_json::json!({
        "full_name": "Juan Perez",
        "headline": "Desarrollador backend",
        "bio": "diez años en sistemas distribuidos",
        "location": "Buenos Aires, AR",
        "links": [{ "type": "github", "url": "https://github.com/juan" }],
        "skills": ["rust", "postgres"]
    });
    let response =
        call_with_credentials(&app, "PUT", "/v1/profile", &access, Some(full.clone())).await;
    assert_eq!(response.status(), StatusCode::OK, "the full put saves");
    let body = json_of(response).await;
    assert_eq!(body["profile"]["headline"], "Desarrollador backend");
    assert_eq!(
        body["profile"]["links"][0]["url"],
        "https://github.com/juan"
    );

    // The put of the name only: the rest EMPTIES (the full-put).
    let response = call_with_credentials(
        &app,
        "PUT",
        "/v1/profile",
        &access,
        Some(serde_json::json!({ "full_name": "Juan" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_of(response).await;
    assert_eq!(body["profile"]["full_name"], "Juan");
    assert!(
        body["profile"].get("headline").is_none(),
        "the omitted (even from the previous put) = emptied"
    );
    assert!(
        body["profile"].get("links").is_none(),
        "the omitted = emptied"
    );

    // The next read agrees (the persistence of the full-put).
    let response = call_with_credentials(&app, "GET", "/v1/profile", &access, None).await;
    let body = json_of(response).await;
    assert_eq!(body["profile"]["full_name"], "Juan");
    assert!(body["profile"].get("headline").is_none());
}

/// The per-field validation (422): the empty name (after the trim) and
/// the limits — the `fields` extension names by field, nothing is saved.
#[tokio::test]
async fn the_invalid_fields_answer_422_by_field_and_save_nothing() {
    let Some((app, _user, access)) = session_with_mirror().await else {
        return;
    };

    let response = call_with_credentials(
        &app,
        "PUT",
        "/v1/profile",
        &access,
        Some(serde_json::json!({
            "full_name": "   ",
            "headline": "x".repeat(121)
        })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json_of(response).await;
    assert!(body["type"]
        .as_str()
        .is_some_and(|t| t.ends_with("profile-invalid-field")));
    assert!(
        body["fields"].get("full_name").is_some(),
        "the name: 1..200 after the trim"
    );
    assert!(
        body["fields"].get("headline").is_some(),
        "the headline: <= 120"
    );

    // Nothing saved: the profile keeps its placeholder.
    let response = call_with_credentials(&app, "GET", "/v1/profile", &access, None).await;
    let body = json_of(response).await;
    assert_eq!(body["profile"]["full_name"], "User", "nothing saved");
    assert!(body["profile"].get("headline").is_none());
}

/// The optimistic precondition (Q2) over the HTTP: the write with the
/// LIVE version passes and ROTATES it; the write with the SPENT version
/// answers 412 `stale-version` with `current_updated_at` and NOTHING was
/// saved. The contract's precision = seconds: the clock leaves the
/// version's second before the comparisons (learned.md's blind spot).
#[tokio::test]
async fn the_optimistic_precondition_arbitrates_the_concurrent_write() {
    let Some((app, _user, access)) = session_with_mirror().await else {
        return;
    };

    let response = call_with_credentials(&app, "GET", "/v1/profile", &access, None).await;
    let body = json_of(response).await;
    let live_version = body["profile"]["updated_at"]
        .as_str()
        .expect("the version")
        .to_string();

    // The write with the LIVE version: the optimistic pass.
    let ok = put_with_precondition(
        &app,
        &access,
        serde_json::json!({ "full_name": "Escritor Vivo" }),
        Some(&live_version),
    )
    .await;
    assert_eq!(ok.status(), StatusCode::OK, "the live version passes");
    let rotated = json_of(ok).await["profile"]["updated_at"]
        .as_str()
        .expect("the rotated version")
        .to_string();

    // The write with the SPENT version (the same instant, the row
    // already changed): the clock makes the difference real (the second
    // leaves) — 412 with the current version, nothing written.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let stale = put_with_precondition(
        &app,
        &access,
        serde_json::json!({ "full_name": "El Que Llego Tarde" }),
        Some(&live_version),
    )
    .await;
    assert_eq!(
        stale.status(),
        StatusCode::PRECONDITION_FAILED,
        "the spent version"
    );
    let body = json_of(stale).await;
    assert!(body["type"]
        .as_str()
        .is_some_and(|t| t.ends_with("stale-version")));
    assert!(
        body["current_updated_at"].as_str().is_some(),
        "the current version names itself"
    );
    assert_eq!(
        body["current_updated_at"],
        serde_json::Value::from(rotated.as_str()),
        "the current = the version of the successful write"
    );

    // Nothing written by the stale try: the profile keeps the value of
    // the optimistic pass.
    let response = call_with_credentials(&app, "GET", "/v1/profile", &access, None).await;
    assert_eq!(
        json_of(response).await["profile"]["full_name"],
        "Escritor Vivo"
    );
}

/// The PUT with the optimistic precondition (the local plumbing: the
/// header does not travel in the harness's helper).
async fn put_with_precondition(
    application: &perfil_api::Application,
    bearer: &str,
    body: serde_json::Value,
    if_unmodified_since: Option<&str>,
) -> axum::response::Response {
    use tower::ServiceExt;
    let mut builder = Request::builder()
        .method("PUT")
        .uri("/v1/profile")
        .header("x-request-id", crate::common::new_request_id())
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    if let Some(instant) = if_unmodified_since {
        builder = builder.header(header::IF_UNMODIFIED_SINCE, instant);
    }
    let request = builder
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("the request is valid");
    application
        .router
        .clone()
        .oneshot(request)
        .await
        .expect("the router never fails")
}
