//! The CV flow E2E (T019, US4): the save (the 201 of the contract, the
//! failure fasts), the read (the text INTEGRO), the archive of the
//! replacements (FR-016), the optimistic precondition (Q2 — nothing
//! travels stale), the anti-BOLA of the versions (FR-019) and the
//! retention closing with its D18 accounting (`cv.versions_purged`,
//! SC-005).
//!
//! CAPABILITY PROBE (learned.md, same of profile_flow): the legacy local
//! stack answers an EMPTY JWKS — its credentials verify nothing: the
//! first protected call decides whether the stack can carry the flow
//! (the CI's modern one does; the legacy documents and ends).
//!
//! The CV-write limit of the process defaults HIGH via the harness (the
//! retention suites chain dozen of replacements inside one window — the
//! R2 cap is for the clients' traffic, not for the fixtures): `the env
//! wins` — an export keeps its word.

use crate::common::{admin_create_user, app, call, json_of, new_request_id};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

/// One request WITH the credential: the bearer travels with the
/// correlation id — the local plumbing of this suite.
async fn call_with_credentials(
    application: &perfil_api::Application,
    method: &str,
    path: &str,
    bearer: &str,
    body: Option<serde_json::Value>,
) -> axum::response::Response {
    let builder = Request::builder()
        .method(method)
        .uri(path)
        .header("x-request-id", new_request_id())
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

/// The save with (or without) the optimistic precondition: the local
/// plumbing — the header does not travel in `call_with_credentials`.
async fn save_cv(
    application: &perfil_api::Application,
    bearer: &str,
    content: &str,
    if_unmodified_since: Option<String>,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/v1/cv/text")
        .header("x-request-id", new_request_id())
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(instant) = if_unmodified_since {
        builder = builder.header(header::IF_UNMODIFIED_SINCE, instant);
    }
    let request = builder
        .body(Body::from(
            serde_json::json!({ "content": content }).to_string(),
        ))
        .expect("the request is valid");
    application
        .router
        .clone()
        .oneshot(request)
        .await
        .expect("the router never fails")
}

/// One request with RAW bytes as body (the encoding's fail fast needs the
/// body the JSON never could carry: bytes that are not UTF-8).
async fn call_raw(
    application: &perfil_api::Application,
    method: &str,
    path: &str,
    bearer: &str,
    raw: Vec<u8>,
) -> axum::response::Response {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("x-request-id", new_request_id())
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(raw))
        .expect("the request is valid");
    application
        .router
        .clone()
        .oneshot(request)
        .await
        .expect("the router never fails")
}

/// A verified session of the fixture (the admin API) + the capability
/// probe on the FIRST protected call. None = the stack cannot carry the
/// flow (the legacy's empty JWKS) — the caller documents and ends.
async fn session() -> Option<(perfil_api::Application, crate::common::TestUser, String)> {
    let app = app().await?;
    let user = admin_create_user(true).await;

    let login = call(
        &app,
        "POST",
        "/v1/auth/login",
        Some(serde_json::json!({ "email": user.email, "password": user.password })),
    )
    .await;
    assert_eq!(login.status(), StatusCode::OK, "the fixture logs in");
    let access = json_of(login).await["access_token"]
        .as_str()
        .expect("the credential")
        .to_string();

    // The probe (the harness's shared one): the stack's gate decides —
    // the legacy documents and ends, a transient doubts, anything else is
    // loud. The suite never FAILS on the machine's stack quirks; the CI
    // carries the flows.
    use crate::common::Probe;
    match crate::common::capability_probe(|| {
        call_with_credentials(&app, "GET", "/v1/cv/text", &access, None)
    })
    .await
    {
        Probe::Carry => {
            // The gate verified and served: the suite proceeds — any later
            // unexpected rejection is the suite's own business (they only
            // occur when the stack itself is ill, which the CI decides).
        }
        Probe::Legacy => {
            eprintln!("the stack verifies nothing through JWKS (legacy): the CV flow pends the CI");
            return None;
        }
        Probe::Undecided => {
            eprintln!("the probe could not decide: the CV flow pends the CI");
            return None;
        }
    }
    Some((app, user, access))
}

/// The 404 of the EMPTY CV, then the first save (201 with
/// `previous_version_archived: false`) and the read travelling the text
/// INTEGRO — the accents, the newlines and the emoji included (FR-015).
#[tokio::test]
async fn the_first_save_answers_201_and_the_read_travels_intact() {
    let Some((app, _user, access)) = session().await else {
        return;
    };

    // GET without a saved CV: the catalogue's not-found — the only 404
    // the contract expects.
    let empty = call_with_credentials(&app, "GET", "/v1/cv/text", &access, None).await;
    assert_eq!(empty.status(), StatusCode::NOT_FOUND, "the empty CV");
    assert!(
        json_of(empty).await["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("not-found")),
        "the slug of the catalogue answers"
    );

    let text = "Juan Pérez\nDesarrollador backend\n\nEXPERIENCIA\n— 2024: Rust, Postgres\n🚀 listo";
    let save = call_with_credentials(
        &app,
        "POST",
        "/v1/cv/text",
        &access,
        Some(serde_json::json!({ "content": text })),
    )
    .await;
    assert_eq!(save.status(), StatusCode::CREATED, "ADR 0007: Created");
    let body = json_of(save).await;
    assert_eq!(
        body["previous_version_archived"], false,
        "FR-016: the first save archives nothing"
    );
    chrono::DateTime::parse_from_rfc3339(body["cv"]["updated_at"].as_str().expect("instant"))
        .expect("the updated is ISO8601");

    // The read: the SAME text, intact.
    let read = call_with_credentials(&app, "GET", "/v1/cv/text", &access, None).await;
    assert_eq!(read.status(), StatusCode::OK);
    let body = json_of(read).await;
    assert_eq!(body["cv"]["content"], text, "the text travels intact");
    chrono::DateTime::parse_from_rfc3339(body["cv"]["created_at"].as_str().expect("instant"))
        .expect("the created is ISO8601");
}

/// FR-016/018: the replacement answers `previous_version_archived: true`,
/// the history holds the anterior version as METADATA (origin `user`, NO
/// content) most recent first, and the per-version read recovers the
/// anterior text exactly.
#[tokio::test]
async fn the_replacement_archives_and_the_history_holds_it() {
    let Some((app, _user, access)) = session().await else {
        return;
    };

    let first = call_with_credentials(
        &app,
        "POST",
        "/v1/cv/text",
        &access,
        Some(serde_json::json!({ "content": "primera redacción del CV" })),
    )
    .await;
    assert_eq!(first.status(), StatusCode::CREATED);

    let second = call_with_credentials(
        &app,
        "POST",
        "/v1/cv/text",
        &access,
        Some(serde_json::json!({ "content": "segunda redacción del CV" })),
    )
    .await;
    assert_eq!(second.status(), StatusCode::CREATED);
    assert_eq!(
        json_of(second).await["previous_version_archived"],
        true,
        "the replacement archives (FR-016)"
    );

    // The listing: metadata only — NEVER the history's contents.
    let list = call_with_credentials(&app, "GET", "/v1/cv/versions", &access, None).await;
    assert_eq!(list.status(), StatusCode::OK);
    let listed = json_of(list).await;
    let versions = listed["versions"].as_array().expect("the listing");
    assert_eq!(versions.len(), 1, "one replacement = one archive");
    assert_eq!(versions[0]["origin"], "user", "the provenance of D15");
    assert!(versions[0].get("content").is_none());
    chrono::DateTime::parse_from_rfc3339(versions[0]["archived_at"].as_str().expect("instant"))
        .expect("the archived is ISO8601");

    // The per-version read: the anterior text travels.
    let version_id = versions[0]["id"].as_str().expect("id").to_string();
    let read = call_with_credentials(
        &app,
        "GET",
        &format!("/v1/cv/versions/{version_id}"),
        &access,
        None,
    )
    .await;
    assert_eq!(read.status(), StatusCode::OK);
    let body = json_of(read).await;
    assert_eq!(body["version"]["content"], "primera redacción del CV");
}

/// The failure fasts over the real stack: the BYTES' cap answers the 413
/// with `max_bytes` (51201 rejected, the 51200 exact ACCEPTED — the spec's
/// edge), and the body that is not UTF-8 answers the 422
/// `cv-invalid-encoding`; the valid UTF-8 that is not the contract's JSON
/// is the 400.
#[tokio::test]
async fn the_bytes_fail_fast_over_the_real_stack() {
    let Some((app, _user, access)) = session().await else {
        return;
    };

    // One byte over: 413 with the catalogue's extension.
    let over = "a".repeat(51201);
    let too_large = call_with_credentials(
        &app,
        "POST",
        "/v1/cv/text",
        &access,
        Some(serde_json::json!({ "content": over })),
    )
    .await;
    assert_eq!(
        too_large.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "FR-015: the cap answers the 413"
    );
    assert_eq!(json_of(too_large).await["max_bytes"], 51200);

    // The exact edge: accepted (the spec's edge case).
    let edge = "a".repeat(51200);
    let ok_edge = call_with_credentials(
        &app,
        "POST",
        "/v1/cv/text",
        &access,
        Some(serde_json::json!({ "content": edge })),
    )
    .await;
    assert_eq!(
        ok_edge.status(),
        StatusCode::CREATED,
        "51200 exact accepted"
    );

    // The encoding: a body not UTF-8 → the 422 of the catalogue (the
    // fail fast's FIRST gate — it never reaches the JSON).
    let invalid = call_raw(&app, "POST", "/v1/cv/text", &access, vec![0xC3, 0x28, 0xFF]).await;
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(json_of(invalid).await["type"]
        .as_str()
        .is_some_and(|t| t.ends_with("cv-invalid-encoding")));

    // The valid UTF-8 that is not the contract's JSON: the 400.
    let not_json = call_raw(&app, "POST", "/v1/cv/text", &access, b"not a json".to_vec()).await;
    assert_eq!(not_json.status(), StatusCode::BAD_REQUEST);
}

/// Q2: the rewrite citing a DESACTUALIZED instant answers the 412
/// `stale-version` with the current version and NOTHING travels (no
/// archive, no overwrite); the precondition that AGREES passes the write
/// (the response's own `updated_at` quotes back accepted).
#[tokio::test]
async fn the_stale_rewrite_answers_412_and_saves_nothing() {
    let Some((app, _user, access)) = session().await else {
        return;
    };

    // First write: no header required.
    let first = save_cv(&app, &access, "CV vigente", None).await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let updated = json_of(first).await["cv"]["updated_at"]
        .as_str()
        .expect("instant")
        .to_string();

    // The precondition that AGREES (the same instant the 201 quoted): the
    // write passes, the anterior content archives.
    let agrees = save_cv(&app, &access, "CV reescrito de vivo", Some(updated.clone())).await;
    assert_eq!(
        agrees.status(),
        StatusCode::CREATED,
        "Q2: the agreeing precondition passes"
    );
    assert_eq!(json_of(agrees).await["previous_version_archived"], true);

    // The STALE cite (two seconds back): the 412 with the current version.
    let stale_instant = chrono::DateTime::parse_from_rfc3339(&updated)
        .expect("the quoted instant parses")
        - chrono::Duration::seconds(2);
    let stale = save_cv(
        &app,
        &access,
        "CV del conflicto",
        Some(stale_instant.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
    )
    .await;
    assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED, "Q2: 412");
    let body = json_of(stale).await;
    assert!(body["type"]
        .as_str()
        .is_some_and(|t| t.ends_with("stale-version")));
    assert!(
        body["current_updated_at"].as_str().is_some(),
        "the extension carries the current version"
    );

    // Nothing travelled stale: the live text stays and only ONE archive
    // exists (the agreeing write's).
    let read = call_with_credentials(&app, "GET", "/v1/cv/text", &access, None).await;
    assert_eq!(json_of(read).await["cv"]["content"], "CV reescrito de vivo");
    let list = call_with_credentials(&app, "GET", "/v1/cv/versions", &access, None).await;
    assert_eq!(
        json_of(list).await["versions"]
            .as_array()
            .expect("list")
            .len(),
        1,
        "the conflict archived NOTHING"
    );
}

/// FR-019 over the real stack: the versions of ANOTHER user never travel
/// — B attacks A's version id → the SAME not-found; B's own CV is the
/// 404 of the empty and B's listing empty. Nothing reveals A's data.
#[tokio::test]
async fn the_versions_of_another_user_never_travel() {
    let Some((app, _user_a, access_a)) = session().await else {
        return;
    };
    let Some((app_same, _user_b, access_b)) = session().await else {
        return;
    };
    let _ = app_same; // (the same application instance carries both)

    call_with_credentials(
        &app,
        "POST",
        "/v1/cv/text",
        &access_a,
        Some(serde_json::json!({ "content": "CV confidencial de A" })),
    )
    .await;
    call_with_credentials(
        &app,
        "POST",
        "/v1/cv/text",
        &access_a,
        Some(serde_json::json!({ "content": "CV segundo de A" })),
    )
    .await;
    let list = call_with_credentials(&app, "GET", "/v1/cv/versions", &access_a, None).await;
    let version_id = json_of(list).await["versions"][0]["id"]
        .as_str()
        .expect("A's history")
        .to_string();

    // B attacks A's version: the 404 that never reveals.
    let stolen = call_with_credentials(
        &app,
        "GET",
        &format!("/v1/cv/versions/{version_id}"),
        &access_b,
        None,
    )
    .await;
    assert_eq!(stolen.status(), StatusCode::NOT_FOUND, "the anti-BOLA");
    assert!(json_of(stolen).await["type"]
        .as_str()
        .is_some_and(|t| t.ends_with("not-found")));

    // B's own view: empty everywhere.
    let empty_text = call_with_credentials(&app, "GET", "/v1/cv/text", &access_b, None).await;
    assert_eq!(empty_text.status(), StatusCode::NOT_FOUND);
    let empty_list = call_with_credentials(&app, "GET", "/v1/cv/versions", &access_b, None).await;
    assert_eq!(
        json_of(empty_list).await["versions"]
            .as_array()
            .expect("list")
            .len(),
        0
    );
}

/// The capture of the events: a MINIMAL subscriber collecting one map per
/// event (the envelope of D18 renders into it). Thread-local by default —
/// the `#[tokio::test]` runs single-threaded, so the emissions of the run
/// land here (the same of auth_ratelimit's suite).
#[derive(Clone, Default)]
struct EventCapture(Arc<Mutex<Vec<serde_json::Value>>>);

struct CaptureVisitor(serde_json::Map<String, serde_json::Value>);

impl tracing::field::Visit for CaptureVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(
            field.name().to_string(),
            serde_json::Value::String(value.into()),
        );
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.insert(
            field.name().to_string(),
            serde_json::Value::String(format!("{value:?}")),
        );
    }
}

impl tracing::Subscriber for EventCapture {
    fn record_follows_from(&self, _in: &tracing::Id, _follows: &tracing::Id) {}
    fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _attributes: &tracing::span::Attributes<'_>) -> tracing::Id {
        tracing::Id::from_u64(1)
    }
    fn record(&self, _span: &tracing::Id, _values: &tracing::span::Record<'_>) {}
    fn enter(&self, _span: &tracing::Id) {}
    fn exit(&self, _span: &tracing::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut visitor = CaptureVisitor(serde_json::Map::new());
        event.record(&mut visitor);
        self.0
            .lock()
            .expect("the capture never poisons")
            .push(serde_json::Value::Object(std::mem::take(&mut visitor.0)));
    }
}

/// SC-005/FR-017 E2E: 12 chained replacements keep exactly 10 versions
/// (the retention purges the surplus in the same tx — nothing half-way)
/// and the D18 emission carries the accounting: `cv.versions_purged` with
/// `purged_count` and the window kept.
#[tokio::test]
async fn the_retention_keeps_ten_and_the_event_carries_the_count() {
    let Some((app, _user, access)) = session().await else {
        return;
    };

    let capture = EventCapture::default();
    let _guard = tracing::subscriber::set_default(capture.clone());
    for round in 1..=12 {
        let response = call_with_credentials(
            &app,
            "POST",
            "/v1/cv/text",
            &access,
            Some(serde_json::json!({ "content": format!("CV {round} del flujo") })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED, "round {round}");
    }

    // The listing: EXACTLY 10, most recent first — content 11 read by id.
    let list = call_with_credentials(&app, "GET", "/v1/cv/versions", &access, None).await;
    let versions = json_of(list).await["versions"]
        .as_array()
        .expect("list")
        .clone();
    assert_eq!(versions.len(), 10, "SC-005: the retention keeps exactly 10");
    let newest = versions[0]["id"].as_str().expect("id").to_string();
    let read = call_with_credentials(
        &app,
        "GET",
        &format!("/v1/cv/versions/{newest}"),
        &access,
        None,
    )
    .await;
    assert_eq!(
        json_of(read).await["version"]["content"],
        "CV 11 del flujo",
        "most recent first (round 12 is the LIVE one)"
    );

    // The D18 accounting: the purge emitted with its count.
    let captured = capture.0.lock().expect("the capture never poisons").clone();
    let seen = captured
        .iter()
        .filter(|event| {
            event["event"]
                .as_str()
                .is_some_and(|name| name == "cv.versions_purged")
        })
        .collect::<Vec<_>>();
    assert!(!seen.is_empty(), "the purge emits its event: {seen:?}");
    let meta: serde_json::Value =
        serde_json::from_str(seen[0]["meta"].as_str().expect("the meta of the envelope"))
            .expect("the meta is JSON");
    assert_eq!(meta["kept"], 10, "the window preserved");
    // 12 saves = 11 archives = exactly 1 fallen version (the 12th save's
    // archive pushed the oldest out of the window of 10).
    let fallen: u64 = seen
        .iter()
        .map(|emission| {
            serde_json::from_str::<serde_json::Value>(
                (*emission)["meta"].as_str().expect("the meta"),
            )
            .expect("JSON meta")["purged_count"]
                .as_u64()
                .expect("purged_count numeric")
        })
        .sum();
    assert_eq!(fallen, 1, "12 saves = 11 archives = 1 fallen");
}
