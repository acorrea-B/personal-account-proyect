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
    use axum::extract::State as AxumState;
    use axum::http::StatusCode;
    use axum::routing::get;
    use axum::Router;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::Value;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

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
        token_with_kid(Some("kid_test"), issuer, aud)
    }

    fn token_with_kid(kid: Option<&str>, issuer: Option<&str>, aud: &str) -> String {
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
        header.kid = kid.map(str::to_string);
        encode(
            &header,
            &claims,
            &EncodingKey::from_ec_pem(PRIV_PEM.as_bytes()).expect("the test key parses"),
        )
        .expect("the test token signs")
    }

    /// The secret of the symmetric fixtures: its base64url (without pad) is
    /// the `k` of the oct keys of `oct_set`.
    const SYMMETRIC_SECRET: &[u8] = b"jwks-test";

    fn hs_token(aud: &str) -> String {
        let claims = serde_json::json!({
            "sub": "c0a8e1f2-33b4-4c5d-8e9f-a0b1c2d3e4f5",
            "aud": aud,
            "exp": 4_102_344_000_000_u64,
        });
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some("kid_test".to_string());
        encode(
            &header,
            &claims,
            &EncodingKey::from_secret(SYMMETRIC_SECRET),
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

    /// A set of one symmetric key (`oct`): `alg` optional — absent, the
    /// key parameters decide the fallback (OctetKey → HS256).
    fn oct_set(alg: Option<&str>) -> JwkSet {
        let mut value = serde_json::json!({
            "keys": [{
                "kty": "oct",
                "k": "andrcy10ZXN0",
                "kid": "kid_test"
            }]
        });
        if let Some(alg) = alg {
            value["keys"][0]["alg"] = serde_json::Value::String(alg.to_owned());
        }
        serde_json::from_value(value).expect("the oct set parses")
    }

    /// The fake of the endpoint JWKS: a server in process (axum is already
    /// a dependency of the API) on an ephemeral port. The mutable body and
    /// status simulate the rotation (another `kid`) and the failures (500,
    /// garbage); the hit counter proves that the cache avoids the refetch.
    #[derive(Clone)]
    struct FakeJwks {
        response: Arc<Mutex<(u16, String)>>,
        hits: Arc<AtomicUsize>,
    }

    impl FakeJwks {
        fn set_response(&self, status: u16, body: &str) {
            *self.response.lock().expect("the fake state never poisons") =
                (status, body.to_string());
        }

        fn hits(&self) -> usize {
            self.hits.load(Ordering::Relaxed)
        }
    }

    async fn serve_jwks(AxumState(fake): AxumState<FakeJwks>) -> (StatusCode, String) {
        fake.hits.fetch_add(1, Ordering::Relaxed);
        let (status, body) = fake
            .response
            .lock()
            .expect("the fake state never poisons")
            .clone();
        (
            StatusCode::from_u16(status).expect("the fake status is valid"),
            body,
        )
    }

    async fn spawn_fake_jwks() -> (FakeJwks, String) {
        let fake = FakeJwks {
            response: Arc::new(Mutex::new((200, SET_JSON.to_string()))),
            hits: Arc::new(AtomicUsize::new(0)),
        };
        let app = Router::new()
            .route("/jwks.json", get(serve_jwks))
            .with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the ephemeral port opens");
        let url = format!(
            "http://{}/jwks.json",
            listener.local_addr().expect("the bind keeps the address")
        );
        tokio::spawn(async move { axum::serve(listener, app).await.expect("the fake serves") });
        (fake, url)
    }

    /// A verifier against the network route (the unit tests of above use
    /// only the synchronous core).
    fn network_verifier(url: &str) -> JwksVerifier {
        JwksVerifier::new(url, None, "authenticated", Duration::from_secs(1))
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

    /// A credential without `kid`: the verifier never guesses the key (401).
    #[tokio::test]
    async fn a_credential_without_kid_rejects() {
        let verifier = verifier(None);
        assert!(matches!(
            verifier.verify_against(&token_with_kid(None, None, "authenticated"), &set()),
            Err(ApiError::Unauthorized { .. })
        ));
    }

    /// The endpoint without `alg` declared: the key parameters decide (EC
    /// P-256 → ES256) — the endpoint does not always declare it.
    #[tokio::test]
    async fn a_key_without_alg_resolves_es256_by_its_parameters() {
        let mut parametered = serde_json::to_value(set()).unwrap();
        parametered["keys"][0]["alg"] = Value::Null;
        let set: JwkSet = serde_json::from_value(parametered).unwrap();
        let verifier = verifier(None);
        assert!(verifier
            .verify_against(&token(None, "authenticated"), &set)
            .is_ok());
    }

    /// The symmetric fallback: with `alg` declared and by parameters (oct →
    /// HS256, the legacy way the HS256 test verifier travels in prod).
    #[tokio::test]
    async fn a_symmetric_key_falls_back_to_hs256() {
        let hs_token = hs_token("authenticated");
        let verifier = verifier(None);
        assert!(verifier
            .verify_against(&hs_token, &oct_set(Some("HS256")))
            .is_ok());
        assert!(verifier.verify_against(&hs_token, &oct_set(None)).is_ok());
    }

    /// A key of the set the library cannot decode (an oct with `k` that is
    /// not base64url): the 500 internal, never a 401 of the world.
    #[tokio::test]
    async fn a_corrupt_key_of_the_set_is_internal() {
        let corrupt: JwkSet =
            serde_json::from_str(r#"{"keys": [{"kty": "oct", "k": "!!!", "kid": "kid_test"}]}"#)
                .expect("the corrupt set parses");
        let verifier = verifier(None);
        assert!(matches!(
            verifier.verify_against(&hs_token("authenticated"), &corrupt),
            Err(ApiError::Internal)
        ));
    }

    /// The cold start of the verifier: the empty cache obliges the fetch of
    /// the endpoint; the second verify with the same `kid` does not touch
    /// the network again (the cache covers the common case).
    #[tokio::test]
    async fn the_cache_covers_the_second_verify() {
        let (fake, url) = spawn_fake_jwks().await;
        let verifier = network_verifier(&url);
        assert!(verifier.verify(&token(None, "authenticated")).await.is_ok());
        assert_eq!(fake.hits(), 1, "the cold start fetches once");
        assert!(verifier.verify(&token(None, "authenticated")).await.is_ok());
        assert_eq!(fake.hits(), 1, "the cache covers the repetition");
    }

    /// The rotation published another `kid`: the cache does not know it →
    /// one refetch, and the verifier resolves the new key without re-deploy.
    #[tokio::test]
    async fn an_unknown_kid_after_a_rotation_refetches() {
        let (fake, url) = spawn_fake_jwks().await;
        let verifier = network_verifier(&url);
        assert!(verifier.verify(&token(None, "authenticated")).await.is_ok());
        let mut rotated = serde_json::to_value(set()).unwrap();
        rotated["keys"][0]["kid"] = Value::String("kid_rotated".into());
        fake.set_response(200, &rotated.to_string());
        assert!(verifier
            .verify(&token_with_kid(Some("kid_rotated"), None, "authenticated"))
            .await
            .is_ok());
        assert_eq!(fake.hits(), 2, "the unknown kid obliges the refetch");
    }

    /// The endpoint answers 500: the failure of the refresh is a dependency
    /// problem (503), never a credential one.
    #[tokio::test]
    async fn an_internal_error_of_the_endpoint_is_a_dependency_outage() {
        let (fake, url) = spawn_fake_jwks().await;
        fake.set_response(500, "outage");
        let verifier = network_verifier(&url);
        assert!(matches!(
            verifier.verify(&token(None, "authenticated")).await,
            Err(ApiError::DependencyUnavailable)
        ));
    }

    /// The endpoint unparsable (200 with garbage): the same dependency 503.
    #[tokio::test]
    async fn an_unparsable_set_is_a_dependency_outage() {
        let (fake, url) = spawn_fake_jwks().await;
        fake.set_response(200, "this-is-not-a-jwks");
        let verifier = network_verifier(&url);
        assert!(matches!(
            verifier.verify(&token(None, "authenticated")).await,
            Err(ApiError::DependencyUnavailable)
        ));
    }

    /// The endpoint off (the connection refused): the same dependency 503.
    #[tokio::test]
    async fn an_off_endpoint_is_a_dependency_outage() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the ephemeral port opens");
        let url = format!(
            "http://{}/jwks.json",
            listener.local_addr().expect("the bind keeps the address")
        );
        drop(listener);
        let verifier = network_verifier(&url);
        assert!(matches!(
            verifier.verify(&token(None, "authenticated")).await,
            Err(ApiError::DependencyUnavailable)
        ));
    }
}
