//! The rate limits of R2 in action (T016, US2): the 6th failed login and
//! the 4th resend of the window → the canonical 429 with `Retry-After`,
//! and the emission of `security.rate_limited` with its route/key.
//!
//! The gate is IN-MEMORY of the app instance: the tests isolate by
//! `X-Forwarded-For` (their own IP each) — the other suites' calls share
//! the "unknown" bucket and never fight here.

use crate::common::{admin_create_user, app, json_of, new_request_id};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};

use chrono::Utc;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

/// One request AS a client IP (the XFF the gate reads) — the isolation of
/// the rate-limit tests.
async fn call_from(
    application: &perfil_api::Application,
    method: &str,
    path: &str,
    ip: &str,
    body: Option<serde_json::Value>,
) -> axum::response::Response {
    let builder = Request::builder()
        .method(method)
        .uri(path)
        .header("x-request-id", new_request_id())
        .header("x-forwarded-for", ip);
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

/// 6 failed logins in the window: the 1st-5th keep their 401; the 6th is
/// the 429 of the catalogue (rate-limited, with `Retry-After` — R2).
#[tokio::test]
async fn the_sixth_failed_login_of_the_window_answers_the_429() {
    let Some(app) = app().await else {
        return;
    };
    admin_create_user(true).await; // (the fixture: the counter keys by IP)
    let ip = format!("203.0.113.{}", Utc::now().timestamp_subsec_nanos() % 200);
    let wrong = serde_json::json!({
        "email": format!("rate-{}@test.local", Utc::now().timestamp()),
        "password": "otra-pass1"
    });

    let mut statuses = Vec::new();
    for _ in 0..6 {
        let response = call_from(&app, "POST", "/v1/auth/login", &ip, Some(wrong.clone())).await;
        statuses.push(response.status());
    }
    for status in &statuses[..5] {
        assert_eq!(
            *status,
            StatusCode::UNAUTHORIZED,
            "the window keeps the 401"
        );
    }
    assert_eq!(
        statuses[5],
        StatusCode::TOO_MANY_REQUESTS,
        "R2: the 6th failed login of the window gates"
    );

    // The canonical block: rate-limited + Retry-After.
    let response = call_from(&app, "POST", "/v1/auth/login", &ip, Some(wrong.clone())).await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after = response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    assert!(
        retry_after.is_some_and(|seconds| seconds >= 1),
        "the Retry-After of the block travels"
    );
    assert!(json_of(response).await["type"]
        .as_str()
        .is_some_and(|t| t.ends_with("rate-limited")));
}

/// 4 resends in the window of one email: the 1st-3rd answer the generic
/// 200 (FR-006); the 4th is the 429 of the catalogue (R2: 3/10min).
#[tokio::test]
async fn the_fourth_resend_of_the_window_answers_the_429() {
    let Some(app) = app().await else {
        return;
    };
    let ip = format!(
        "203.0.113.{}",
        200 + (Utc::now().timestamp_subsec_nanos() % 200)
    );
    let email = format!("resend-rate-{}@test.local", Utc::now().timestamp());
    let body = serde_json::json!({ "email": email });

    for _ in 0..3 {
        let response = call_from(
            &app,
            "POST",
            "/v1/auth/resend-verification",
            &ip,
            Some(body.clone()),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "the window keeps the generic 200"
        );
    }
    let response = call_from(
        &app,
        "POST",
        "/v1/auth/resend-verification",
        &ip,
        Some(body),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "R2: the 4th resend of the window gates"
    );
    assert!(json_of(response).await["type"]
        .as_str()
        .is_some_and(|t| t.ends_with("rate-limited")));
}

/// The capture of the events: a MINIMAL subscriber collecting one map per
/// event (the envelope of D18 renders into it). Thread-local by default —
/// the `#[tokio::test]` runs single-threaded, so the layer's emissions of
/// the same run land here.
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

/// The D18 emission of the block: `security.rate_limited` with the blocked
/// outcome, the route and the key of the counter.
#[tokio::test]
async fn the_blocked_emission_carries_route_and_key() {
    let Some(app) = app().await else {
        return;
    };
    let ip = format!(
        "203.0.113.{}",
        400 + (Utc::now().timestamp_subsec_nanos() % 200)
    );
    let email = format!("emission-rate-{}@test.local", Utc::now().timestamp());
    let body = serde_json::json!({ "email": email });

    let capture = EventCapture::default();
    let _guard = tracing::subscriber::set_default(capture.clone());
    for _ in 0..4 {
        let _ = call_from(
            &app,
            "POST",
            "/v1/auth/resend-verification",
            &ip,
            Some(body.clone()),
        )
        .await;
    }
    drop(_guard);
    let seen = capture.0.lock().expect("the capture never poisons").clone();

    let blocked: Vec<&serde_json::Value> = seen
        .iter()
        .filter(|event| {
            event["event"]
                .as_str()
                .is_some_and(|name| name == "security.rate_limited")
        })
        .collect();
    assert!(!blocked.is_empty(), "the block emits its event: {seen:?}");
    for emission in &blocked {
        assert_eq!((*emission)["outcome"], "blocked", "the blocked outcome");
        // The meta travels stringified (the envelope's field): parse it.
        let meta: serde_json::Value = serde_json::from_str(
            (*emission)["meta"]
                .as_str()
                .expect("the meta of the envelope"),
        )
        .expect("the meta is JSON");
        assert_eq!(meta["route"], "/v1/auth/resend-verification");
        // The resend keys by the email when the body carries it (the
        // label of the layer): the key = "user" (the email's bucket); the
        // ip travels in the envelope's ip field for the same event.
        assert_eq!(meta["key"], "user", "this gate keys by the email");
    }
}
