//! HS256 verification of the Supabase credentials (R1), the implementation
//! behind the `SignatureVerifier` trait.
//!
//! The `SUPABASE_JWT_SECRET` is the shared secret of the flow documented:
//! the alternative route (JWKS URL of Supabase, rotation without re-deploy)
//! implements the same trait and replaces this type in the state — the
//! wiring and the catalogue of answers do not change.

use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};

use crate::errors::ApiError;
use crate::middleware::auth::{Claims, SignatureVerifier};

/// Verifier HS256: `firma → exp → iss/aud` (R1).
pub struct Hs256Verifier {
    decoding_key: DecodingKey,
    validation: Validation,
}

impl Hs256Verifier {
    pub fn new(secret: &str, issuer: &str, audience: &str) -> Self {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[issuer.to_string()]);
        validation.set_audience(&[audience.to_string()]);
        Self {
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
            validation,
        }
    }
}

impl SignatureVerifier for Hs256Verifier {
    fn verify(&self, token: &str) -> Result<Claims, ApiError> {
        let inner = match decode::<Claims>(token, &self.decoding_key, &self.validation) {
            Ok(verification) => verification.claims,
            Err(err) => {
                return Err(map_error(err.kind().clone()));
            }
        };

        Ok(inner)
    }
}

/// The library errors map to the canonical slugs without any internal
/// technical message leaking (§13.4).
fn map_error(error: ErrorKind) -> ApiError {
    match error {
        // The client can recover it by renewing: token-expired is not
        // a simple unauthorized.
        ErrorKind::ExpiredSignature => ApiError::TokenExpired {
            detail: "The credential expired; renew the session.".to_string(),
        },
        ErrorKind::InvalidSignature | ErrorKind::InvalidToken => ApiError::Unauthorized {
            detail: "The credential is invalid.".to_string(),
        },
        ErrorKind::InvalidIssuer | ErrorKind::InvalidAudience => ApiError::Unauthorized {
            detail: "The credential is invalid for this service.".to_string(),
        },
        _ => {
            // The verification failed in an unforeseen way (alg confusion,
            // claim missing...): the fixed generic detail does not leak.
            tracing::warn!(error_kind = ?error, "the credential could not be verified");
            ApiError::Internal
        }
    }
}
#[cfg(test)]
impl Hs256Verifier {
    /// Issues a test credential: `expired` builds one with `exp` in the
    /// past to exercise the mapping of the expiry.
    pub fn test_token(app: &crate::state::AppState, user_id: &str, expired: bool) -> String {
        use jsonwebtoken::{encode, EncodingKey, Header};

        let exp = if expired {
            1_000_000
        } else {
            4_102_344_000_000_u64
        };
        encode(
            &Header::new(Algorithm::HS256),
            &serde_json::json!({
                "sub": user_id,
                "iss": app.jwt_issuer,
                "aud": app.jwt_audience,
                "exp": exp
            }),
            &EncodingKey::from_secret(app.jwt_secret.as_bytes()),
        )
        .expect("the test token is valid")
    }
}
