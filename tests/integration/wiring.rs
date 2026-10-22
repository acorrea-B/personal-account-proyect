//! Wiring test (T010): the 10 API routes exist and answer 401/503
//! according to their class BEFORE the domains land. The local Supabase is
//! not strictly needed (no DB hit: the stubs return before the handlers);
//! the skip guard runs anyway (the harness needs it for the fixtures).

use crate::common::{app, call, json_of};

use axum::http::StatusCode;

/// The public routes (FR-011): the wiring answers 503 with the canonical
/// document (the stub) — the credential is not required.
#[tokio::test]
async fn the_public_routes_answer_503_from_the_stub() {
    let Some(app) = app().await else {
        return;
    };

    for (method, path) in [
        ("POST", "/v1/auth/register"),
        ("POST", "/v1/auth/login"),
        ("POST", "/v1/auth/refresh"),
        ("POST", "/v1/auth/resend-verification"),
    ] {
        let response = call(&app, method, path, Some(serde_json::json!({}))).await;
        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "{method} {path} must exist in the router"
        );
        let document = json_of(response).await;
        assert_eq!(
            document["type"], "https://api.example.com/errors/dependency-unavailable",
            "{method} {path} must answer the canonical stub"
        );
    }

    // The health probe stays public (the smoke test depends on it).
    let response = call(&app, "GET", "/health", None).await;
    assert_eq!(response.status(), StatusCode::OK);
}

/// The protected routes (FR-011 deny-by-default): 401 without a
/// credential — the gate answers before the stub.
#[tokio::test]
async fn the_protected_routes_require_a_credential() {
    let Some(app) = app().await else {
        return;
    };

    for (method, path) in [
        ("POST", "/v1/auth/revoke"),
        ("GET", "/v1/profile"),
        ("PUT", "/v1/profile"),
        ("POST", "/v1/cv/text"),
        ("GET", "/v1/cv/text"),
        ("GET", "/v1/cv/versions"),
        (
            "GET",
            "/v1/cv/versions/019c0eb3-4c8e-7dd2-b358-e1a5c66d1f4d",
        ),
    ] {
        let response = call(&app, method, path, None).await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {path} must be protected (deny-by-default)"
        );
        let document = json_of(response).await;
        assert_eq!(
            document["type"],
            "https://api.example.com/errors/unauthorized"
        );
        assert_eq!(document["instance"], path);
    }
}
