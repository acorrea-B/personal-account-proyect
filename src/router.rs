//! Route mapping and the explicit list of public routes (FR-011, deny-by-default).
//!
//! The public/protected split is explicit: the public routes hang of the
//! root router; the protected ones take the `AuthenticatedUser` extractor —
//! any route missing from `PUBLIC_ROUTES` requires a valid credential.
//!
//! The 10 API handlers are still stubs that answer 503 (T010 wiring): the
//! domains and their handlers land by user story (T011+) and replace each
//! stub.

use axum::{
    routing::{get, post, put},
    Router,
};

use crate::domain::auth;
use crate::errors::{ApiError, Problem};
use crate::middleware::auth::AuthenticatedUser;
use crate::state::AppState;

use auth::handlers;

/// Public API routes (FR-011). Anything not listed here requires a
/// credential.
pub const PUBLIC_ROUTES: &[&str] = &[
    "POST /v1/auth/register",
    "POST /v1/auth/resend-verification",
    "POST /v1/auth/login",
    "POST /v1/auth/refresh",
    "GET /health",
];

/// Health probe for the post-deploy smoke test (plan §12.2).
async fn health() -> &'static str {
    "ok"
}

/// The stub of the protected routes until their handlers land (T012+): 503
/// with the canonical document (the wiring test proves the route exists
/// and the gate answered — not the handler logic).
async fn stub_protected_503(_user: AuthenticatedUser) -> Problem {
    Problem::new(ApiError::DependencyUnavailable)
}

/// The stub of the public routes until their handlers land (T011+).
async fn stub_public_503() -> Problem {
    Problem::new(ApiError::DependencyUnavailable)
}

/// The stub of the version route (its `id` is a parameter).
async fn stub_version_503(_version_id: axum::extract::Path<String>) -> Problem {
    Problem::new(ApiError::DependencyUnavailable)
}

pub fn build_router(state: AppState) -> Router {
    // Protected: every handler takes `AuthenticatedUser`, so an anonymous
    // request is rejected before reaching the handler (deny-by-default).
    let protected = Router::new()
        .route("/v1/auth/revoke", post(stub_protected_503))
        .route("/v1/profile", get(stub_protected_503))
        .route("/v1/profile", put(stub_protected_503))
        .route("/v1/cv/text", post(stub_protected_503))
        .route("/v1/cv/text", get(stub_protected_503))
        .route("/v1/cv/versions", get(stub_protected_503))
        .route("/v1/cv/versions/{id}", get(stub_version_503))
        .with_state(state.clone());

    Router::new()
        .route("/health", get(health))
        // The register flow (T012, US1) — the login/refresh stay stubs until
        // T015.
        .route("/v1/auth/register", post(handlers::register))
        .route(
            "/v1/auth/resend-verification",
            post(handlers::resend_verification),
        )
        .route("/v1/auth/login", post(stub_public_503))
        .route("/v1/auth/refresh", post(stub_public_503))
        .merge(protected)
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::hs256::Hs256Verifier;
    use crate::state::AppState;

    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    /// The protected route without a credential is denied by default
    /// (FR-011): 401 with the canonical problem document.
    #[tokio::test]
    async fn the_protected_route_without_token_answers_401_unauthorized() {
        let router = build_router(AppState::for_tests());

        let response = router
            .oneshot(
                Request::get("/v1/profile")
                    .body(Body::empty())
                    .expect("the request is valid"),
            )
            .await
            .expect("the router never fails");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/problem+json")
        );

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("complete");
        let document: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
        assert_eq!(
            document["type"],
            "https://api.example.com/errors/unauthorized"
        );
        assert_eq!(document["instance"], "/v1/profile");
    }

    /// With a valid HS256 credential the gate lets the request pass: the
    /// stub answers 503 (the wiring exists; the handler arrives with T018).
    #[tokio::test]
    async fn the_protected_route_with_a_valid_credential_reaches_the_stub() {
        let state = AppState::for_tests();
        let router = build_router(state.clone());
        let token =
            Hs256Verifier::test_token(&state, "c0a8e1f2-33b4-4c5d-8e9f-a0b1c2d3e4f5", false);

        let response = router
            .oneshot(
                Request::get("/v1/profile")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("the request is valid"),
            )
            .await
            .expect("the router never fails");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("complete");
        let document: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
        assert_eq!(
            document["type"],
            "https://api.example.com/errors/dependency-unavailable"
        );
    }

    /// The version route (with `id` in the path) with a valid credential
    /// reaches its own stub: the parameter wiring answered 503 too.
    #[tokio::test]
    async fn the_version_route_with_a_valid_credential_reaches_the_stub() {
        let state = AppState::for_tests();
        let router = build_router(state.clone());
        let token =
            Hs256Verifier::test_token(&state, "c0a8e1f2-33b4-4c5d-8e9f-a0b1c2d3e4f5", false);

        let response = router
            .oneshot(
                Request::get("/v1/cv/versions/019c0eb3-4c8e-7dd2-b358-e1a5c66d1f4d")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("the request is valid"),
            )
            .await
            .expect("the router never fails");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("complete");
        let document: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
        assert_eq!(
            document["type"],
            "https://api.example.com/errors/dependency-unavailable"
        );
    }

    /// The public route of a stub answers 503 — no credential required
    /// (FR-011).
    #[tokio::test]
    async fn the_public_stub_answers_503_without_credential() {
        let router = build_router(AppState::for_tests());

        let response = router
            .oneshot(
                Request::post("/v1/auth/login")
                    .body(Body::empty())
                    .expect("the request is valid"),
            )
            .await
            .expect("the router never fails");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/problem+json")
        );
    }

    /// The expired credential is never a generic unauthorized: the renewal
    /// is the documented recovery (the middleware maps it to token-expired).
    #[tokio::test]
    async fn the_expired_credential_answers_token_expired() {
        let state = AppState::for_tests();
        let router = build_router(state.clone());
        let expired =
            Hs256Verifier::test_token(&state, "c0a8e1f2-33b4-4c5d-8e9f-a0b1c2d3e4f5", true);

        let response = router
            .oneshot(
                Request::get("/v1/profile")
                    .header(header::AUTHORIZATION, format!("Bearer {expired}"))
                    .body(Body::empty())
                    .expect("the request is valid"),
            )
            .await
            .expect("the router never fails");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("complete");
        let document: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
        assert_eq!(
            document["type"],
            "https://api.example.com/errors/token-expired"
        );
    }
}
