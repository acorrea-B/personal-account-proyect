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
