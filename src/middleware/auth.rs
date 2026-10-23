//! Authentication gate (D3/R1): verification of the Supabase credential.
//!
//! Deny-by-default (FR-011): a protected route only exists if its handler
//! takes the `AuthenticatedUser` extractor — the credential is validated
//! firma → exp → iss/aud and the request resolves to one identity or it is
//! answered with the canonical error, never to an ambiguous anonymous user.
//!
//! The signature verification is the `SignatureVerifier` trait (R1): the
//! HS256 implementation of `hs256.rs` with `SUPABASE_JWT_SECRET` is behind
//! the trait; the documented alternative (JWKS URL) plugs in the same
//! point. The implementation is injected in the `AppState`.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use serde::Deserialize;
use uuid::Uuid;

use crate::errors::{ApiError, Problem};
use crate::state::AppState;

/// Verifies the signature and the claims of a credential of use (R1).
/// Async: the JWKS verifier can refresh the key set on an unknown kid.
#[async_trait::async_trait]
pub trait SignatureVerifier: Send + Sync {
    /// Validates the token and resolves the identity it certifies.
    async fn verify(&self, token: &str) -> Result<Claims, ApiError>;
}

/// Claims of the credential of use after the library validation: `sub` is
/// the user id (D3). `exp`, `iss` and `aud` are validated by the verifier.
#[derive(Debug, Deserialize)]
pub struct Claims {
    pub sub: String,
}

/// Extractor of the authenticated identity (deny-by-default, FR-011): a
/// handler that takes this extractor rejects the request with the canonical
/// error 401 while there is no valid credential.
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user_id: Uuid,
}

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Problem> {
        let token = match authorization_token(parts) {
            Some(token) => token,
            None => {
                return Err(Problem::new(ApiError::Unauthorized {
                    detail: "The Authorization header is missing.".to_string(),
                })
                .on(request_path(parts)));
            }
        };

        let claims = state
            .signature_verifier
            .verify(&token)
            .await
            .map_err(|api_error| Problem::new(api_error).on(request_path(parts)))?;

        claims
            .sub
            .parse::<Uuid>()
            .map(|user_id| AuthenticatedUser { user_id })
            .map_err(|_| {
                // A non-uuid `sub` is not an identity of the project: it is
                // rejected by default.
                Problem::new(ApiError::Unauthorized {
                    detail: "The credential does not carry a valid identity.".to_string(),
                })
                .on(request_path(parts))
            })
    }
}

/// Reads `Authorization: Bearer <token>` of the request.
fn authorization_token(parts: &Parts) -> Option<String> {
    let header = parts.headers.get(axum::http::header::AUTHORIZATION)?;
    let header = header.to_str().ok()?;
    let (scheme, token) = header.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") {
        Some(token.trim().to_string())
    } else {
        None
    }
}

/// The route of the request: the `instance` of the problem document.
fn request_path(parts: &Parts) -> String {
    parts.uri.path().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::hs256::Hs256Verifier;
    use crate::router::build_router;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header as JwtHeader};
    use tower::ServiceExt;

    /// A credential signed ok whose `sub` is not an identity of the
    /// project (not a UUID): deny-by-default — unauthorized, never an
    /// anonymous pass nor an internal.
    #[tokio::test]
    async fn a_credential_without_a_valid_identity_rejects_unauthorized() {
        let claims = serde_json::json!({
            "sub": "no-es-una-identidad",
            "aud": "authenticated",
            "exp": 4_102_344_000_000_u64,
        });
        let token = encode(
            &JwtHeader::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret("test-jwt-secret".as_bytes()),
        )
        .expect("the test token signs");

        let router = build_router(crate::state::AppState::for_tests());
        let response = router
            .oneshot(
                Request::get("/v1/profile")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("the request is valid"),
            )
            .await
            .expect("the router never fails");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(response.into_body(), 1024)
            .await
            .expect("the problem reads");
        let document: serde_json::Value =
            serde_json::from_slice(&body).expect("the problem is JSON");
        assert!(document["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("unauthorized")));
    }

    /// The `Authorization` header of a scheme that is not bearer: the
    /// credential was not presented — the same unauthorized of the gate.
    #[tokio::test]
    async fn a_non_bearer_scheme_rejects_unauthorized() {
        let router = build_router(crate::state::AppState::for_tests());
        let response = router
            .oneshot(
                Request::get("/v1/profile")
                    .header(header::AUTHORIZATION, "Basic c29tZXRoaW5n")
                    .body(Body::empty())
                    .expect("the request is valid"),
            )
            .await
            .expect("the router never fails");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// The raw call of the extractor through the parts: the expired
    /// credential maps `token-expired` (the renewal is the documented
    /// recovery — the middleware, not a generic unauthorized).
    #[tokio::test]
    async fn the_expired_credential_of_the_extractor_maps_token_expired() {
        use axum::extract::FromRequestParts;
        use axum::response::IntoResponse;

        let state = crate::state::AppState::for_tests();
        let expired =
            Hs256Verifier::test_token(&state, "c0a8e1f2-33b4-4c5d-8e9f-a0b1c2d3e4f5", true);
        let request = Request::get("/v1/profile")
            .header(header::AUTHORIZATION, format!("Bearer {expired}"))
            .body(())
            .expect("the request builds");
        let (mut parts, _) = request.into_parts();

        let problem = AuthenticatedUser::from_request_parts(&mut parts, &state)
            .await
            .expect_err("the expired credential rejects");
        let response = problem.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(response.into_body(), 1024)
            .await
            .expect("the problem reads");
        let document: serde_json::Value =
            serde_json::from_slice(&body).expect("the problem is JSON");
        assert!(document["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("token-expired")));
    }
}
