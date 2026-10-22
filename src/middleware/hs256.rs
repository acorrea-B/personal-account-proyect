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
    /// `issuer`: Some = the credentials always carry it and must coincide;
    /// None = the issuer of Supabase is not emitted (the legacy gotrue
    /// tokens have no iss claim) — the verification skips it.
    pub fn new(secret: &str, issuer: Option<&str>, audience: &str) -> Self {
        let mut validation = Validation::new(Algorithm::HS256);
        if let Some(issuer) = issuer {
            validation.set_issuer(&[issuer.to_string()]);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};

    /// Issues a credential for the tests of this module: `with_iss` adds the
    /// claim of the issuer (absent in the legacy gotrue tokens).
    fn token(secret: &str, iss: Option<&str>) -> String {
        let claims = serde_json::json!({
            "sub": "c0a8e1f2-33b4-4c5d-8e9f-a0b1c2d3e4f5",
            "aud": "authenticated",
            "exp": 4_102_344_000_000_u64,
            "iss": iss,
        });
        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("the test token is valid")
    }

    /// The issuer absent (legacy gotrue — T006 amend) skips its check: the
    /// credential without `iss` is valid for the verifier without issuer.
    #[test]
    fn the_verifier_without_issuer_accepts_a_token_without_iss() {
        let verifier = Hs256Verifier::new("test-secret", None, "authenticated");
        assert!(verifier.verify(&token("test-secret", None)).is_ok());
    }

    /// The issuer declared must coincide: the mismatch is never a leakage
    /// of the internal cause — plain unauthorized (§13.4).
    #[test]
    fn the_verifier_rejects_a_token_of_another_issuer() {
        let verifier = Hs256Verifier::new("test-secret", Some("supabase"), "authenticated");
        assert!(matches!(
            verifier.verify(&token("test-secret", Some("other-tenant"))),
            Err(ApiError::Unauthorized { .. })
        ));
    }
}
