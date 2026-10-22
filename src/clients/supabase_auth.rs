//! REST client of Supabase Auth (T008): signup, login, renewal, resend and
//! revocation, with the acotated retries of R7 (timeout 3s, 2 retries with
//! fixed backoff for 5xx or network fall) and the mapping of the Supabase
//! error codes to the catalogue slugs (rule 1 of D18: one vocabulary of
//! reasons).
//!
//! The project uses the new key system of Supabase: `sb_publishable_...`
//! for the public endpoints and `sb_secret_...` (service role) for the
//! admin — with the legacy keys the same two slots take the
//! `anon`/`service_role`. FR-009 (revoke of the sessions of the user of
//! the token) resolves with `POST /auth/v1/logout?scope=…`, the documented
//! endpoint that kills the current or every session of the token's user.

use std::time::Duration;

use reqwest::StatusCode;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::errors::ApiError;

/// How the request authenticates against Supabase: the publishable key
/// (public flows) or the service key (admin).
#[derive(Debug, Clone, Copy)]
enum Auth {
    Publishable,
    // The admin flows of the service key (deletion of the account D12,
    // session admin) land with their tasks; the slot exists for the new
    // key system of Supabase.
    #[allow(dead_code)]
    Service,
}

/// The credential pair the session flows exchange (FR-008): access token of
/// use and refresh token of renewal.
#[derive(Debug, Clone)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
}

/// The REST client of Supabase Auth.
#[derive(Clone)]
pub struct SupabaseAuthClient {
    http: reqwest::Client,
    base_url: String,
    publishable_key: String,
    service_key: String,
    /// Fixed acotation of the transient retries (R7).
    retries: u32,
}

impl SupabaseAuthClient {
    pub fn new(
        base_url: &str,
        publishable_key: &str,
        service_key: &str,
        timeout: Duration,
    ) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .expect("the reqwest client of Supabase always builds"),
            base_url: base_url.trim_end_matches('/').to_string(),
            publishable_key: publishable_key.to_string(),
            service_key: service_key.to_string(),
            retries: 2,
        }
    }

    /// `POST /auth/v1/signup` (FR-003): resolves the id of the user; NO
    /// session travels here — the tokens never leave the session flow.
    pub async fn signup(&self, email: &str, password: &str) -> Result<Uuid, ApiError> {
        let response = self
            .request(
                reqwest::Method::POST,
                "/auth/v1/signup",
                Auth::Publishable,
                Some(json!({ "email": email, "password": password })),
            )
            .await?;

        response["id"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or(ApiError::Internal)
    }

    /// `POST /auth/v1/token?grant_type=password` (FR-001): login accepted
    /// or the canonical mapping (`invalid-credentials`, `email-not-verified`).
    pub async fn sign_in_with_password(
        &self,
        email: &str,
        password: &str,
    ) -> Result<TokenPair, ApiError> {
        let response = self
            .request(
                reqwest::Method::POST,
                "/auth/v1/token?grant_type=password",
                Auth::Publishable,
                Some(json!({ "email": email, "password": password })),
            )
            .await?;
        tokens_of(&response)
    }

    /// `POST /auth/v1/token?grant_type=refresh_token` (FR-008): renewal of
    /// the session (`invalid-refresh-token` | `refresh-token-reused`).
    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenPair, ApiError> {
        let response = self
            .request(
                reqwest::Method::POST,
                "/auth/v1/token?grant_type=refresh_token",
                Auth::Publishable,
                Some(json!({ "refresh_token": refresh_token })),
            )
            .await?;
        tokens_of(&response)
    }

    /// `POST /auth/v1/resend` (FR-005): the resend of the confirmation;
    /// the handler decides the generic answer (FR-006).
    pub async fn resend_verification(&self, email: &str) -> Result<(), ApiError> {
        self.request(
            reqwest::Method::POST,
            "/auth/v1/resend",
            Auth::Publishable,
            Some(json!({ "type": "signup", "email": email })),
        )
        .await
        .map(|_| ())
    }

    /// `POST /auth/v1/logout` with the access token of the user (FR-009):
    /// `global` kills every session of the user, `local` only the current.
    pub async fn revoke(&self, access_token: &str, global: bool) -> Result<(), ApiError> {
        let scope = if global { "global" } else { "local" };
        self.request_with(
            reqwest::Method::POST,
            &format!("/auth/v1/logout?scope={scope}"),
            Auth::Publishable,
            Some(json!({})),
            Some(access_token),
        )
        .await
        .map(|_| ())
    }

    /// One request with the retries of R7: only the network failures and
    /// the 5xx retry (2 acotados, backoff fixed); anything else resolves
    /// in the first response.
    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        auth: Auth,
        body: Option<serde_json::Value>,
    ) -> Result<Value, ApiError> {
        self.request_with(method, path, auth, body, None).await
    }

    async fn request_with(
        &self,
        method: reqwest::Method,
        path: &str,
        auth: Auth,
        body: Option<serde_json::Value>,
        bearer_override: Option<&str>,
    ) -> Result<Value, ApiError> {
        let key = match auth {
            Auth::Publishable => self.publishable_key.clone(),
            Auth::Service => self.service_key.clone(),
        };

        for attempt in 0..=self.retries {
            let mut request = self
                .http
                .request(method.clone(), format!("{}{path}", self.base_url))
                .header("apikey", &key);

            // reqwest appends the values of the repeated header instead of
            // replacing them: only one Authorization travels — the user's
            // token (logout) or the key.
            request = match bearer_override {
                Some(bearer) => {
                    request.header(reqwest::header::AUTHORIZATION, format!("Bearer {bearer}"))
                }
                None => request.header(reqwest::header::AUTHORIZATION, format!("Bearer {key}")),
            };
            if let Some(body) = &body {
                request = request.json(body);
            }

            match request.send().await {
                Err(err) => {
                    // Network failure: retriable but acotado (R7).
                    if attempt == self.retries {
                        tracing::warn!(error = %err, "Supabase did not answer");
                        return Err(ApiError::DependencyUnavailable);
                    }
                    tokio::time::sleep(backoff(attempt)).await;
                }
                Ok(response) => {
                    let status = response.status();
                    if status.as_u16() >= 500 {
                        // Transient of Supabase: retriable but acotado (R7).
                        if attempt == self.retries {
                            return Err(ApiError::DependencyUnavailable);
                        }
                        tokio::time::sleep(backoff(attempt)).await;
                        continue;
                    }

                    let bytes = response
                        .bytes()
                        .await
                        .map_err(|_| ApiError::DependencyUnavailable)?;
                    if status.is_success() {
                        return Ok(serde_json::from_slice(&bytes)
                            .unwrap_or_else(|_| Value::Object(Default::default())));
                    }

                    // 4xx: the body carries {code, error_code, msg}.
                    let document: Value = serde_json::from_slice(&bytes).unwrap_or_default();
                    return Err(auth_error_of(status, &document));
                }
            }
        }

        // Unreachable: the loop always returns before the last attempt ends.
        Err(ApiError::Internal)
    }
}

/// Backoff of the retries: simple exponential (150ms, 450ms).
fn backoff(attempt: u32) -> Duration {
    Duration::from_millis((150 * (attempt + 1) * 3) as u64)
}

/// The credential pair of a session answer (FR-008).
fn tokens_of(response: &Value) -> Result<TokenPair, ApiError> {
    match (
        response["access_token"].as_str(),
        response["refresh_token"].as_str(),
    ) {
        (Some(access), Some(refresh)) if !access.is_empty() && !refresh.is_empty() => {
            Ok(TokenPair {
                access_token: access.to_string(),
                refresh_token: refresh.to_string(),
            })
        }
        // A 200 without credentials is a Supabase misconfiguration: nothing
        // of the contract travels (FR-003 remains enforced).
        _ => Err(ApiError::Internal),
    }
}

/// Mapping of the Supabase error codes to the closed catalogue
/// (contracts/errors.md). The unknown codes keep the generic internal
/// detail and their code stays in the log only (§13.4).
fn auth_error_of(status: StatusCode, document: &Value) -> ApiError {
    let code = document["error_code"].as_str().unwrap_or_default();
    match code {
        "invalid_credentials" => ApiError::InvalidCredentials,
        "email_not_confirmed" => ApiError::EmailNotVerified,
        "user_already_exists" => ApiError::EmailExists,
        "refresh_token_not_found" => ApiError::InvalidRefreshToken {
            detail: "The refresh token does not exist or expired.".to_string(),
        },
        "refresh_token_already_used" => ApiError::RefreshTokenReused,
        "weak_password" => ApiError::WeakPassword,
        "email_address_invalid" | "validation_failed" => ApiError::EmailMalformed,
        "over_request_rate_limit" | "over_email_send_rate_limit" => ApiError::RateLimited {
            retry_after_seconds: 60,
        },
        _ => {
            tracing::warn!(error_code = %code, status = %status, "unmapped Supabase error");
            ApiError::Internal
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// One canned HTTP server per case: it answers with the given response
    /// (constant or per attempt), counts the hits (retries of R7) and
    /// records the (path, authorization) of each request.
    #[allow(clippy::type_complexity)]
    async fn canned(
        per_request: impl Fn(usize) -> axum::response::Response + Send + Sync + 'static,
    ) -> (
        String,
        Arc<AtomicU32>,
        Arc<std::sync::Mutex<Vec<(String, String)>>>,
    ) {
        use std::sync::Mutex as StdMutex;

        let hits = Arc::new(AtomicU32::new(0));
        let captured: Arc<StdMutex<Vec<(String, String)>>> = Arc::default();
        let counter = hits.clone();
        let seen = captured.clone();
        let per_request = Arc::new(per_request);

        let app = axum::Router::new().fallback(
            move |request: axum::extract::Request<axum::body::Body>| {
                let hit = counter.fetch_add(1, Ordering::SeqCst) as usize;
                let per_request = std::sync::Arc::clone(&per_request);
                let seen = std::sync::Arc::clone(&seen);
                async move {
                    let authorization = request
                        .headers()
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    seen.lock().expect("the capture never poisons").push((
                        request
                            .uri()
                            .path_and_query()
                            .map(|p| p.to_string())
                            .unwrap_or_default(),
                        authorization,
                    ));
                    per_request(hit)
                }
            },
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a local port always binds");
        let address = listener.local_addr().expect("local address visible");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("the server runs");
        });
        (format!("http://{address}"), hits, captured)
    }

    fn client(base: &str) -> SupabaseAuthClient {
        SupabaseAuthClient::new(
            base,
            "test-publishable",
            "test-service-key",
            Duration::from_secs(3),
        )
    }

    use axum::response::IntoResponse;

    fn problem(status: StatusCode, body: Value) -> axum::response::Response {
        (status, Json(body)).into_response()
    }

    /// `user_already_exists` → `email-exists` (FR-002).
    #[tokio::test]
    async fn the_signup_maps_user_already_exists() {
        let (base, _hits, _seen) = canned(move |_| {
            problem(StatusCode::from_u16(422).expect("valid"), serde_json::json!({
                "code": 422, "error_code": "user_already_exists", "msg": "User already registered"
            }))
        })
        .await;

        let error = client(&base)
            .signup("user@mail.com", "Passw0rd1")
            .await
            .unwrap_err();
        assert_eq!(error.slug(), "email-exists");
    }

    /// A 200 of signup resolves the user id (FR-003: no session travels).
    #[tokio::test]
    async fn the_signup_resolves_the_user_id() {
        let (base, _hits, _seen) = canned(move |_| {
            problem(
                StatusCode::OK,
                serde_json::json!({
                    "id": "c0a8e1f2-33b4-4c5d-8e9f-a0b1c2d3e4f5", "email": "user@mail.com"
                }),
            )
        })
        .await;

        let id = client(&base)
            .signup("user@mail.com", "Passw0rd1")
            .await
            .unwrap();
        assert_eq!(id.to_string(), "c0a8e1f2-33b4-4c5d-8e9f-a0b1c2d3e4f5");
    }

    /// `invalid_credentials` → the same slug, identical (FR-006);
    /// `email_not_confirmed` → `email-not-verified` with `can_resend`.
    #[tokio::test]
    async fn login_maps_the_contract_errors() {
        let (base, _hits, _seen) = canned(move |_| {
            problem(StatusCode::from_u16(400).expect("valid"), serde_json::json!({
                "code": 400, "error_code": "invalid_credentials", "msg": "Invalid login credentials"
            }))
        })
        .await;
        let result = client(&base)
            .sign_in_with_password("user@mail.com", "Passw0rd1")
            .await;
        assert_eq!(result.unwrap_err().slug(), "invalid-credentials");

        let (base, _hits, _seen) = canned(move |_| {
            problem(
                StatusCode::from_u16(400).expect("valid"),
                serde_json::json!({
                    "code": 400, "error_code": "email_not_confirmed", "msg": "Email not confirmed"
                }),
            )
        })
        .await;
        let result = client(&base)
            .sign_in_with_password("user@mail.com", "Passw0rd1")
            .await;
        assert_eq!(result.unwrap_err().slug(), "email-not-verified");
    }

    /// The renewal: `refresh_token_not_found` → `invalid-refresh-token`;
    /// the reuse detection (`refresh_token_already_used`, FR-008) →
    /// `refresh-token-reused`. A successful renewal rotates the pair.
    #[tokio::test]
    async fn the_renewal_maps_the_contract_errors() {
        let (base, _hits, _seen) = canned(move |_| {
            problem(StatusCode::from_u16(400).expect("valid"), serde_json::json!({
                "code": 400, "error_code": "refresh_token_not_found", "msg": "Invalid Refresh Token"
            }))
        })
        .await;
        let result = client(&base).refresh("stale").await;
        assert_eq!(result.unwrap_err().slug(), "invalid-refresh-token");

        let (base, _hits, _seen) = canned(move |_| {
            problem(StatusCode::from_u16(400).expect("valid"), serde_json::json!({
                "code": 400, "error_code": "refresh_token_already_used", "msg": "Invalid Refresh Token"
            }))
        })
        .await;
        let result = client(&base).refresh("reused").await;
        assert_eq!(result.unwrap_err().slug(), "refresh-token-reused");

        let (base, _hits, _seen) = canned(move |_| {
            problem(
                StatusCode::OK,
                serde_json::json!({
                    "access_token": "new-access", "refresh_token": "new-refresh"
                }),
            )
        })
        .await;
        let pair = client(&base).refresh("valid").await.unwrap();
        assert_eq!(pair.access_token, "new-access");
        assert_eq!(pair.refresh_token, "new-refresh");
    }

    /// The retries (R7): a 5xx the first time does not fail the call —
    /// the second attempt succeeds and the client recovers.
    #[tokio::test]
    async fn a_transient_5xx_retries_and_succeeds() {
        let (base, hits, _seen) = canned(|attempt| {
            if attempt < 1 {
                problem(
                    StatusCode::from_u16(503).expect("valid"),
                    serde_json::json!({"error": "upstream"}),
                )
            } else {
                problem(
                    StatusCode::OK,
                    serde_json::json!({
                        "access_token": "access", "refresh_token": "refresh"
                    }),
                )
            }
        })
        .await;

        let pair = client(&base)
            .sign_in_with_password("user@mail.com", "Passw0rd1")
            .await
            .unwrap();
        assert_eq!(pair.access_token, "access");
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    /// The transiency exhausted (R7: 2 retries): the client answers
    /// `dependency-unavailable`; 1 original + 2 = 3 hits.
    #[tokio::test]
    async fn a_persistent_5xx_ends_in_dependency_unavailable() {
        let (base, hits, _seen) = canned(|_| {
            problem(
                StatusCode::from_u16(502).expect("valid"),
                serde_json::json!({"error": "down"}),
            )
        })
        .await;

        let result = client(&base)
            .sign_in_with_password("user@mail.com", "Passw0rd1")
            .await;
        assert_eq!(result.unwrap_err().slug(), "dependency-unavailable");
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    /// The revocation (FR-009) signs with the token of the user and hits
    /// the scope asked for (local or global).
    #[tokio::test]
    async fn the_revocation_signs_with_the_users_token() {
        let (base, _hits, seen) =
            canned(move |_| problem(StatusCode::NO_CONTENT, serde_json::json!({}))).await;

        let client = client(&base);
        client
            .revoke("access-token-of-the-user", true)
            .await
            .unwrap();
        client
            .revoke("access-token-of-the-user", false)
            .await
            .unwrap();

        let logged = seen.lock().expect("the capture never poisons");
        assert_eq!(logged.len(), 2);
        assert_eq!(
            logged[0],
            (
                "/auth/v1/logout?scope=global".to_string(),
                "Bearer access-token-of-the-user".to_string()
            )
        );
        assert_eq!(
            logged[1],
            (
                "/auth/v1/logout?scope=local".to_string(),
                "Bearer access-token-of-the-user".to_string()
            )
        );
    }
}
