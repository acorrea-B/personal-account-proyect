//! Route mapping and the explicit list of public routes (FR-011, deny-by-default).
//!
//! The public/protected split is explicit: the public routes hang of the
//! root router; the protected ones take the `AuthenticatedUser` extractor —
//! any route missing from `PUBLIC_ROUTES` requires a valid credential.
//!
//! All the handlers are wired over their domains (T012/T015 US1-2, T017
//! US3, T021 US4): no stub remains — the 503 travels only from the real
//! dependency failures.

use axum::{
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Router,
};

use crate::domain::{auth, cv, profile};
use crate::errors::{ApiError, Problem};
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

pub fn build_router(state: AppState) -> Router {
    // Protected: every handler takes `AuthenticatedUser`, so an anonymous
    // request is rejected before reaching the handler (deny-by-default).
    let protected = Router::new()
        .route("/v1/auth/revoke", post(handlers::revoke))
        // The profile flow (T018, US3) and the CV flow (T021, US4): the
        // five route groups of the contract — every handler takes the
        // credential's user only (D3).
        .route("/v1/profile", get(profile::handlers::get_profile))
        .route("/v1/profile", put(profile::handlers::put_profile))
        .route(
            "/v1/cv/text",
            post(cv::handlers::post_text).get(cv::handlers::get_text),
        )
        .route("/v1/cv/versions", get(cv::handlers::list_versions))
        .route("/v1/cv/versions/{id}", get(cv::handlers::get_version))
        .with_state(state.clone());

    Router::new()
        .route("/health", get(health))
        // The register flow (T012) and the session flow (T015, US2) — the
        // revoke stays the protected stub until the session-revocation
        // design (plan §16.6) resolves.
        .route("/v1/auth/register", post(handlers::register))
        .route(
            "/v1/auth/resend-verification",
            post(handlers::resend_verification),
        )
        .route("/v1/auth/login", post(handlers::login))
        .route("/v1/auth/refresh", post(handlers::refresh))
        .merge(protected)
        .with_state(state)
        // Every route that misses answers the catalogue's not-found —
        // NEVER axum's empty body (the contract speaks problem+json for
        // everything; the unknown route too).
        .fallback(unknown_route)
}

/// The unknown route: the catalogue's not-found with the requested path
/// as the `instance` — the contract's document, never a bare 404.
async fn unknown_route(axum::extract::OriginalUri(uri): axum::extract::OriginalUri) -> Response {
    Problem::new(ApiError::NotFound {
        detail: "The route does not exist.".to_string(),
    })
    .on(uri.path())
    .into_response()
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

    /// The public route without the contract body: the wired handlers
    /// (T012/T015) answer the 400 canonical — never a plain text of axum
    /// (FR-011 keeps the routes public).
    #[tokio::test]
    async fn the_public_route_without_body_answers_the_400_canonical() {
        let router = build_router(AppState::for_tests());

        for path in ["/v1/auth/register", "/v1/auth/login"] {
            let response = router
                .clone()
                .oneshot(
                    Request::post(path)
                        .body(Body::empty())
                        .expect("the request is valid"),
                )
                .await
                .expect("the router never fails");

            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "route {path}");
            assert_eq!(
                response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok()),
                Some("application/problem+json"),
                "route {path}"
            );
        }
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

    /// The ROUTE that misses (a typo, the client's invented query — the
    /// versions' trailing-slash included): the catalogue's not-found with
    /// the real path as the instance — never axum's empty body.
    #[tokio::test]
    async fn an_unknown_route_answers_the_catalogue_not_found() {
        let router = build_router(AppState::for_tests());

        let response = router
            .oneshot(
                Request::get("/v1/cv/versions/?%3Aid=758c7b30")
                    .body(Body::empty())
                    .expect("the request is valid"),
            )
            .await
            .expect("the router never fails");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/problem+json")
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("complete");
        let document: serde_json::Value = serde_json::from_slice(&body).expect("problem JSON");
        assert!(
            document["type"]
                .as_str()
                .is_some_and(|t| t.ends_with("not-found")),
            "the catalogue's slug answers"
        );
        assert_eq!(document["instance"], "/v1/cv/versions/");
    }
}
