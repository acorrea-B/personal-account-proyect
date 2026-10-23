//! ES256 verification against the JWKS set of Supabase (the alternative
//! route of R1 — and the current one: the platform signs with asymmetric
//! keys since the october 2025 rollout, the legacy HS256 secret no longer
//! exists in the new projects). The public keys publish in
//! `/auth/v1/.well-known/jwks.json`; a rotation only publishes a new `kid`
//! — the cache re-fetches on an unknown one, without re-deploy.

use std::sync::Mutex;
use std::time::Duration;

use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet, KeyAlgorithm};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};

use async_trait::async_trait;

use crate::errors::ApiError;
use crate::middleware::auth::{Claims, SignatureVerifier};
use crate::middleware::hs256::map_error;

/// Verifier ES256 via JWKS: `firma → exp → iss/aud`, the key resolves by
/// the `kid` of the credential header.
pub struct JwksVerifier {
    jwks_url: String,
    audience: String,
    issuer_to_verify: Option<String>,
    client: reqwest::Client,
    cache: Mutex<Option<JwkSet>>,
}

impl JwksVerifier {
    /// `timeout` bounds the refresh of the set (the reach of Supabase,
    /// `SUPABASE_TIMEOUT_SECS` in the configuration).
    pub fn new(
        jwks_url: &str,
        issuer_to_verify: Option<&str>,
        audience: &str,
        timeout: Duration,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("the reqwest builder of the jwks never fails");
        Self {
            jwks_url: jwks_url.to_string(),
            audience: audience.to_string(),
            issuer_to_verify: issuer_to_verify.map(str::to_string),
            client,
            cache: Mutex::new(None),
        }
    }

    async fn refresh(&self) -> Result<(), ApiError> {
        let response = self
            .client
            .get(&self.jwks_url)
            .send()
            .await
            .map_err(|_| ApiError::DependencyUnavailable)?;
        let set: JwkSet = response
            .error_for_status()
            .map_err(|_| ApiError::DependencyUnavailable)?
            .json()
            .await
            .map_err(|_| ApiError::DependencyUnavailable)?;
        *self.cache.lock().expect("the jwks cache never poisons") = Some(set);
        Ok(())
    }

    /// The verification against one concrete set: the synchronous core,
    /// testable without HTTP.
    fn verify_against(&self, token: &str, set: &JwkSet) -> Result<Claims, ApiError> {
        let kid = decode_header(token)
            .map_err(|err| map_error(err.kind().clone()))?
            .kid
            .ok_or(ApiError::Unauthorized {
                detail: "The credential is invalid.".to_string(),
            })?;
        let jwk = set
            .keys
            .iter()
            .find(|jwk| jwk.common.key_id.as_deref() == Some(kid.as_str()))
            .ok_or(ApiError::Unauthorized {
                detail: "The credential is invalid.".to_string(),
            })?;

        // The algorithm of the key decides (the endpoint does not always
        // declare it — the key parameters are the truth).
        let algorithm = match jwk.common.key_algorithm {
            Some(KeyAlgorithm::ES256) => Algorithm::ES256,
            Some(KeyAlgorithm::HS256) => Algorithm::HS256,
            _ => match &jwk.algorithm {
                AlgorithmParameters::EllipticCurve(_) => Algorithm::ES256,
                _ => Algorithm::HS256,
            },
        };

        let mut validation = Validation::new(algorithm);
        validation.set_audience(&[self.audience.as_str()]);
        if let Some(issuer) = &self.issuer_to_verify {
            validation.set_issuer(&[issuer.as_str()]);
            // set_issuer does not make the claim required by itself: a
            // credential without iss would pass silently otherwise.
            validation.set_required_spec_claims(&["exp", "iss", "aud"]);
        }

        let key = DecodingKey::from_jwk(jwk).map_err(|_| ApiError::Internal)?;

        decode::<Claims>(token, &key, &validation)
            .map(|verification| verification.claims)
            .map_err(|err| map_error(err.kind().clone()))
    }
}

#[async_trait]
impl SignatureVerifier for JwksVerifier {
    /// The cached set covers the common case without network: an unknown
    /// `kid` re-fetches once (rotation), a fetch failure is a dependency
    /// problem — not a credential problem (503, no 401).
    async fn verify(&self, token: &str) -> Result<Claims, ApiError> {
        let kid = decode_header(token)
            .map_err(|err| map_error(err.kind().clone()))?
            .kid;
        let cached = self
            .cache
            .lock()
            .expect("the jwks cache never poisons")
            .clone();
        if let Some(set) = cached {
            if kid.as_deref().is_some_and(|kid| {
                set.keys
                    .iter()
                    .any(|jwk| jwk.common.key_id.as_deref() == Some(kid))
            }) {
                return self.verify_against(token, &set);
            }
        }
        self.refresh().await?;
        let set = self
            .cache
            .lock()
            .expect("the jwks cache never poisons")
            .clone()
            .ok_or(ApiError::DependencyUnavailable)?;
        self.verify_against(token, &set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::Value;

    /// Fixture pair (EC P-256), inline by rule — the repository never
    /// carries a key file: this one is disposable (test-only, signs
    /// nothing real) and the private never leaves this constant. The
    /// private signs, the public travels in the fake set; `kid_test` is
    /// the kid the credentials carry.
    const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
        MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgPSzfPTmrTFoRjsAw\n\
        xMtEJ9/YHjVb0gxybbEdjhQTOL6hRANCAASw8jOFq6WeBeX91Rg4BhHrqc4k7zW4\n\
        tfwWSWKc3t4bPk5jcLGTPOoD9T1K8xq3f2v0vSHKZSFg0zpEV/2DupTp\n\
        -----END PRIVATE KEY-----\n";
    const SET_JSON: &str = r#"{
        "keys": [{
            "kty": "EC",
            "crv": "P-256",
            "kid": "kid_test",
            "alg": "ES256",
            "use": "sig",
            "x": "sPIzhaulngXl_dUYOAYR66nOJO81uLX8FklinN7eGz4",
            "y": "TmNwsZM86gP1PUrzGrd_a_S9IcplIWDTOkRX_YO6lOk"
        }]
    }"#;

    fn token(issuer: Option<&str>, aud: &str) -> String {
        // The issuer absent travels without the key (the explicit null is
        // not the same credential shape as a missing claim).
        let mut claims = serde_json::json!({
            "sub": "c0a8e1f2-33b4-4c5d-8e9f-a0b1c2d3e4f5",
            "aud": aud,
            "exp": 4_102_344_000_000_u64,
        });
        if let Some(issuer) = issuer {
            claims["iss"] = serde_json::Value::String(issuer.to_string());
        }
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("kid_test".to_string());
        encode(
            &header,
            &claims,
            &EncodingKey::from_ec_pem(PRIV_PEM.as_bytes()).expect("the test key parses"),
        )
        .expect("the test token signs")
    }

    fn set() -> JwkSet {
        serde_json::from_str(SET_JSON).expect("the fake set parses")
    }

    fn verifier(issuer: Option<&str>) -> JwksVerifier {
        JwksVerifier::new(
            "https://fake.example/jwks",
            issuer,
            "authenticated",
            Duration::from_secs(1),
        )
    }

    /// The accepted credential: signature + exp + aud, verified against
    /// the asymmetric key without symmetric secret.
    #[tokio::test]
    async fn the_es256_credential_of_the_set_verifies() {
        let verifier = verifier(None);
        assert!(verifier
            .verify_against(&token(None, "authenticated"), &set())
            .is_ok());
    }

    /// The issuer declared must coincide (the cloud carries the URL of
    /// auth in the iss).
    #[tokio::test]
    async fn the_verifier_enforces_the_issuer_when_one_is_declared() {
        let verifier = verifier(Some("https://fake.example/auth/v1"));
        assert!(matches!(
            verifier.verify_against(&token(None, "authenticated"), &set()),
            Err(ApiError::Unauthorized { .. })
        ));
        assert!(verifier
            .verify_against(
                &token(Some("https://fake.example/auth/v1"), "authenticated"),
                &set()
            )
            .is_ok());
    }

    /// A credential of another key (kid absent from the set) never passes.
    #[tokio::test]
    async fn a_credential_of_an_unknown_kid_rejects() {
        let verifier = verifier(None);
        let mut forged = serde_json::to_value(set()).unwrap();
        forged["keys"][0]["kid"] = Value::String("another_kid".into());
        let other_set: JwkSet = serde_json::from_value(forged).unwrap();
        assert!(matches!(
            verifier.verify_against(&token(None, "authenticated"), &other_set),
            Err(ApiError::Unauthorized { .. })
        ));
    }

    /// The wrong audience rejects: the token is signed, but not ours.
    #[tokio::test]
    async fn a_credential_of_another_audience_rejects() {
        let verifier = verifier(None);
        assert!(matches!(
            verifier.verify_against(&token(None, "another_service"), &set()),
            Err(ApiError::Unauthorized { .. })
        ));
    }
}
