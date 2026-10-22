//! Route mapping and the explicit list of public routes (FR-011, deny-by-default).
//!
//! The public/protected split is explicit: public routes are mounted on their
//! own router; protected ones hang from the router with the JWT gate applied.
//! Any route missing from `PUBLIC_ROUTES` requires a valid credential.

use axum::{routing::get, Router};

use crate::middleware::auth::AuthenticatedUser;
use crate::state::AppState;

/// Public API routes (FR-011). Anything not listed here requires a
/// credential: they hang from the protected router whose handlers take the
/// `AuthenticatedUser` extractor.
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

/// Placeholder of a protected route until T018 lands the real handler of
/// `GET /v1/profile`: it exists only so the gate is exercised and tests
/// prove the deny-by-default (FR-011).
async fn stub_profile(user: AuthenticatedUser) -> String {
    user.user_id.to_string()
}

pub fn build_router(state: AppState) -> Router {
    // Protected routes: every handler takes `AuthenticatedUser`, so an
    // anonymous request is rejected before reaching the handler.
    let protected = Router::new()
        .route("/v1/profile", get(stub_profile))
        .with_state(state.clone());

    Router::new()
        .route("/health", get(health))
        .merge(protected)
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;

    use crate::middleware::hs256::Hs256Verifier;
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
            .expect("the body is complete");
        let document: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
        assert_eq!(
            document["type"],
            "https://api.example.com/errors/unauthorized"
        );
        assert_eq!(document["instance"], "/v1/profile");
    }

    /// With a valid HS256 credential (firma→exp→iss/aud all pass) the
    /// protected route answers 200 with the identity resolved (D3).
    #[tokio::test]
    async fn the_protected_route_accepts_a_valid_credential() {
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

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body is complete");
        assert_eq!(body, "c0a8e1f2-33b4-4c5d-8e9f-a0b1c2d3e4f5");
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
            .expect("the body is complete");
        let document: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
        assert_eq!(
            document["type"],
            "https://api.example.com/errors/token-expired"
        );
    }
}
