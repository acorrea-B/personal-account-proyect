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

/// The complete session that Supabase answers to the session flows
/// (login/renewal): the pair with its TTL and the public face of the user
/// (FR-003: no more columns than id/email/verified/created_at) — the
/// contract responses of login/renovation echo it verbatim.
#[derive(Debug, Clone)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub user_id: Uuid,
    pub user_email: String,
    /// Supabase stamps the instant of the confirmation (`email_confirmed_at`
    /// / `confirmed_at`): its presence is the verified flag.
    pub verified: bool,
    pub user_created_at: String,
}

/// The REST client of Supabase Auth.
#[derive(Clone)]
pub struct SupabaseAuthClient {
    http: reqwest::Client,
    base_url: String,
    /// The gateway prefix of the auth routes: EMPTY in the standard
    /// deployment (the paths carry `/auth/v1` inline); `/auth/v1` means
    /// RAW gotrue — the base ends in the prefix and each path STRIPS it
    /// (the raw gotrue serves `/signup` and `/token`, without the gate).
    auth_prefix: &'static str,
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
        let base_url = base_url.trim_end_matches('/');
        let auth_prefix = if base_url.ends_with("/auth/v1") {
            "/auth/v1"
        } else {
            ""
        };
        // The raw base loses its prefix too: the routes resolve at its root.
        let base_url = base_url.strip_suffix("/auth/v1").unwrap_or(base_url);
        Self {
            http: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .expect("the reqwest client of Supabase always builds"),
            base_url: base_url.to_string(),
            auth_prefix,
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
    ) -> Result<Session, ApiError> {
        let response = self
            .request(
                reqwest::Method::POST,
                "/auth/v1/token?grant_type=password",
                Auth::Publishable,
                Some(json!({ "email": email, "password": password })),
            )
            .await?;
        session_of(&response)
    }

    /// `POST /auth/v1/token?grant_type=refresh_token` (FR-008): renewal of
    /// the session (`invalid-refresh-token` | `refresh-token-reused`).
    pub async fn refresh(&self, refresh_token: &str) -> Result<Session, ApiError> {
        let response = self
            .request(
                reqwest::Method::POST,
                "/auth/v1/token?grant_type=refresh_token",
                Auth::Publishable,
                Some(json!({ "refresh_token": refresh_token })),
            )
            .await?;
        session_of(&response)
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

    /// `GET /auth/v1/user` with the access token of the credential (the
    /// session-state check of the plan §16.6, option b): the session of a
    /// verifiable token can still be revoked or the user deleted — goTruth
    /// rejects here; the retries of R7 for the transient 5xx/network and
    /// the `RateLimited` pass through. The other 4xx = the session is NOT
    /// alive (the caller renders `token-revoked` — FR-010).
    pub async fn session_alive(&self, access_token: &str) -> Result<bool, ApiError> {
        match self
            .request_with(
                reqwest::Method::GET,
                "/auth/v1/user",
                Auth::Publishable,
                None,
                Some(access_token),
            )
            .await
        {
            Ok(_) => Ok(true),
            Err(error @ (ApiError::RateLimited { .. } | ApiError::DependencyUnavailable)) => {
                Err(error)
            }
            Err(_) => Ok(false),
        }
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
            // The raw gotrue removes the gateway prefix of the path (the
            // paths carry it inline); the standard deployment keeps it.
            let path = if self.auth_prefix.is_empty() {
                path
            } else {
                path.trim_start_matches("/auth/v1")
            };
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

/// The full session of a session answer (login/renewal): pair + TTL + the
/// public face of the user. A 200 without the contract shape is a Supabase
/// misconfiguration: internal (§13.4), never partial credentials.
fn session_of(response: &Value) -> Result<Session, ApiError> {
    let access = response["access_token"]
        .as_str()
        .filter(|value| !value.is_empty());
    let refresh = response["refresh_token"]
        .as_str()
        .filter(|value| !value.is_empty());
    let expires = response["expires_in"].as_u64();
    let user = &response["user"];
    let id = user["id"].as_str().and_then(|id| Uuid::parse_str(id).ok());
    let email = user["email"].as_str().filter(|value| !value.is_empty());
    let created = user["created_at"]
        .as_str()
        .filter(|value| !value.is_empty());

    if let (Some(access), Some(refresh), Some(expires), Some(id), Some(email), Some(created)) =
        (access, refresh, expires, id, email, created)
    {
        // Supabase stamps the instant of the confirmation when the account
        // verifies; either of the two fields (the legacy/local naming) works.
        return Ok(Session {
            access_token: access.to_string(),
            refresh_token: refresh.to_string(),
            expires_in: expires,
            user_id: id,
            user_email: email.to_string(),
            verified: user["email_confirmed_at"].as_str().is_some()
                || user["confirmed_at"].as_str().is_some(),
            user_created_at: created.to_string(),
        });
    }
    Err(ApiError::Internal)
}

/// Mapping of the Supabase error codes to the closed catalogue
/// (contracts/errors.md). The unknown codes keep the generic internal
/// detail and their code stays in the log only (§13.4).
///
/// Two dialects of goTrue travel here: the MODERN shape
/// (`code`/`error_code`/`msg`) and the LEGACY one of the older stacks
/// (`error`/`error_description` — e.g. `invalid_grant` + "Invalid login
/// credentials" for the failed login, `Invalid Refresh Token: …` for the
/// renewal) — one vocabulary to the catalogue either way.
fn auth_error_of(status: StatusCode, document: &Value) -> ApiError {
    let code = document["error_code"].as_str().unwrap_or_default();
    if !code.is_empty() {
        return modern_error_of(code, status, document);
    }
    let error = document["error"].as_str().unwrap_or_default();
    if !error.is_empty() {
        return legacy_error_of(
            error,
            document["error_description"].as_str().unwrap_or_default(),
            status,
        );
    }
    tracing::warn!(status = %status, "a Supabase error without a recognized shape");
    ApiError::Internal
}

/// The modern shape (T008).
fn modern_error_of(code: &str, status: StatusCode, _document: &Value) -> ApiError {
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

/// The legacy shape (the older goTrue stacks): `invalid_grant` carries the
/// meaning in the description ("Invalid login credentials" vs the Refresh
/// Token family); the rest have a 1:1 name.
fn legacy_error_of(error: &str, description: &str, status: StatusCode) -> ApiError {
    match error {
        "invalid_grant" | "invalid_credentials" => {
            if description.contains("Email not confirmed") {
                ApiError::EmailNotVerified
            } else if description.contains("Refresh Token") {
                if description.contains("Already Used") {
                    ApiError::RefreshTokenReused
                } else {
                    ApiError::InvalidRefreshToken {
                        detail: "The refresh token does not exist or expired.".to_string(),
                    }
                }
            } else {
                ApiError::InvalidCredentials
            }
        }
        "email_not_confirmed" => ApiError::EmailNotVerified,
        "user_already_registered" => ApiError::EmailExists,
        "weak_password" => ApiError::WeakPassword,
        "validation_failed" => ApiError::EmailMalformed,
        "over_request_rate_limit" | "over_email_send_rate_limit" => ApiError::RateLimited {
            retry_after_seconds: 60,
        },
        _ => {
            tracing::warn!(error = %error, status = %status, "unmapped legacy Supabase error");
            ApiError::Internal
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use serde_json::json;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// The user that the 200 responses of the fixtures carry: the minimal
    /// shape of the confirmation done (the Session parse requires it).
    fn session_user() -> serde_json::Value {
        json!({
            "id": "3f2504e0-4f89-41d3-9a0c-0305e82c3301",
            "email": "user@mail.com",
            "email_confirmed_at": "2026-01-01T00:00:00Z",
            "created_at": "2026-01-01T00:00:00Z"
        })
    }

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
                    "access_token": "new-access", "refresh_token": "new-refresh", "expires_in": 3600, "user": session_user()
                }),
            )
        })
        .await;
        let session = client(&base).refresh("valid").await.unwrap();
        assert_eq!(session.access_token, "new-access");
        assert_eq!(session.refresh_token, "new-refresh");
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
                        "access_token": "access", "refresh_token": "refresh", "expires_in": 3600, "user": session_user()
                    }),
                )
            }
        })
        .await;

        let session = client(&base)
            .sign_in_with_password("user@mail.com", "Passw0rd1")
            .await
            .unwrap();
        assert_eq!(session.access_token, "access");
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

    /// The LEGACY dialect maps the SAME catalogue (the failed login of
    /// `invalid_grant` → the identical 401 of SC-004).
    #[tokio::test]
    async fn the_legacy_dialect_of_the_failed_login_maps_the_catalogue() {
        let (base, _hits, _seen) = canned(move |_| {
            problem(
                StatusCode::from_u16(400).expect("valid"),
                serde_json::json!({
                    "error": "invalid_grant", "error_description": "Invalid login credentials"
                }),
            )
        })
        .await;
        let result = client(&base)
            .sign_in_with_password("user@mail.com", "Passw0rd1")
            .await;
        assert_eq!(result.unwrap_err().slug(), "invalid-credentials");
    }

    /// The legacy `Email not confirmed` (inside `invalid_grant`):
    /// `email-not-verified` — the 403 of the gate, not the generic 401.
    #[tokio::test]
    async fn the_legacy_dialect_distinguishes_the_not_confirmed_email() {
        let (base, _hits, _seen) = canned(move |_| {
            problem(
                StatusCode::from_u16(400).expect("valid"),
                serde_json::json!({
                    "error": "invalid_grant", "error_description": "Email not confirmed"
                }),
            )
        })
        .await;
        let result = client(&base)
            .sign_in_with_password("user@mail.com", "Passw0rd1")
            .await;
        assert_eq!(result.unwrap_err().slug(), "email-not-verified");
    }

    /// The legacy renewal: the reuse ("Already Used") and the absence —
    /// the two reasons closed of the renewal, out of the SAME `invalid_grant`.
    #[tokio::test]
    async fn the_legacy_dialect_of_the_renewal_maps_the_two_reasons() {
        let (base, _hits, _seen) = canned(move |_| {
            problem(
                StatusCode::from_u16(400).expect("valid"),
                serde_json::json!({
                    "error": "invalid_grant",
                    "error_description": "Invalid Refresh Token: Already Used"
                }),
            )
        })
        .await;
        let result = client(&base).refresh("the-reused").await;
        assert_eq!(result.unwrap_err().slug(), "refresh-token-reused");

        let (base, _hits, _seen) = canned(move |_| {
            problem(
                StatusCode::from_u16(400).expect("valid"),
                serde_json::json!({
                    "error": "invalid_grant",
                    "error_description": "Invalid Refresh Token: Refresh Token Not Found"
                }),
            )
        })
        .await;
        let result = client(&base).refresh("the-unknown").await;
        assert_eq!(result.unwrap_err().slug(), "invalid-refresh-token");
    }

    /// The legacy signup: the duplicate (`user_already_registered`) and the
    /// weak password — the register's 409/422 of the catalogue.
    #[tokio::test]
    async fn the_legacy_dialect_of_the_signup_maps_the_register_errors() {
        for (body, slug) in [
            (
                serde_json::json!({
                    "error": "user_already_registered", "error_description": "User already registered"
                }),
                "email-exists",
            ),
            (
                serde_json::json!({
                    "error": "weak_password", "error_description": "Password should be at least 6 characters."
                }),
                "weak-password",
            ),
            (
                serde_json::json!({
                    "error": "validation_failed", "error_description": "Unable to validate email address"
                }),
                "email-malformed",
            ),
        ] {
            let (base, _hits, _seen) =
                canned(move |_| problem(StatusCode::from_u16(400).expect("valid"), body.clone()))
                    .await;
            let result = client(&base).signup("user@mail.com", "Passw0rd1").await;
            assert_eq!(result.unwrap_err().slug(), slug);
        }
    }

    /// The modern rate limit (the two names of goTruth) → `rate-limited`
    /// with the Retry-After of the catalogue.
    #[tokio::test]
    async fn the_rate_limit_of_the_modern_shape_maps_with_retry_after() {
        for name in ["over_request_rate_limit", "over_email_send_rate_limit"] {
            let (base, _hits, _seen) = canned(move |_| {
                problem(
                    StatusCode::from_u16(429).expect("valid"),
                    serde_json::json!({ "code": 429, "error_code": name, "msg": "rate-limited" }),
                )
            })
            .await;
            let result = client(&base)
                .sign_in_with_password("user@mail.com", "Passw0rd1")
                .await;
            assert_eq!(result.unwrap_err().slug(), "rate-limited");
        }
    }

    /// An error document WITHOUT a recognized shape (either dialect): the
    /// generic internal with the warn of the log (§13.4).
    #[tokio::test]
    async fn an_error_without_shape_resolves_internal() {
        let (base, _hits, _seen) = canned(move |_| {
            problem(
                StatusCode::from_u16(400).expect("valid"),
                serde_json::json!({}),
            )
        })
        .await;
        let result = client(&base)
            .sign_in_with_password("user@mail.com", "Passw0rd1")
            .await;
        assert_eq!(result.unwrap_err().slug(), "internal");
    }

    /// The unreachable Supabase (the network dead): the retries of R7
    /// exhaust and the `dependency-unavailable` ends the call (never an
    /// internal, never a credential).
    #[tokio::test]
    async fn the_unreachable_supabase_ends_in_dependency_unavailable() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the ephemeral port opens");
        let address = listener.local_addr().expect("the bind keeps it");
        drop(listener);

        let result = client(&format!("http://{address}"))
            .sign_in_with_password("user@mail.com", "Passw0rd1")
            .await;
        assert_eq!(result.unwrap_err().slug(), "dependency-unavailable");
    }

    /// The parse of the session: the legacy name of the confirmation
    /// (`confirmed_at` — without the modern field) verifies too; the 200
    /// WITHOUT the contract shape is a Supabase misconfiguration (internal,
    /// never partial credentials — FR-003).
    #[tokio::test]
    async fn the_session_parse_holds_the_two_confirmation_names() {
        let (base, _hits, _seen) = canned(move |_| {
            problem(
                StatusCode::OK,
                serde_json::json!({
                    "access_token": "the-access", "refresh_token": "the-refresh",
                    "expires_in": 3600,
                    "user": {
                        "id": "3f2504e0-4f89-41d3-9a0c-0305e82c3301",
                        "email": "user@mail.com",
                        "confirmed_at": "2026-01-01T00:00:00Z",
                        "created_at": "2026-01-01T00:00:00Z"
                    }
                }),
            )
        })
        .await;
        let session = client(&base)
            .sign_in_with_password("user@mail.com", "Passw0rd1")
            .await
            .unwrap();
        assert!(session.verified, "the legacy name confirms too");

        // The misconfiguration: a 200 without the shape (no tokens) —
        // internal, never a half-session (FR-003 enforces).
        let (base, _hits, _seen) = canned(move |_| {
            problem(
                StatusCode::OK,
                serde_json::json!({ "message": "something else travels here" }),
            )
        })
        .await;
        let result = client(&base)
            .sign_in_with_password("user@mail.com", "Passw0rd1")
            .await;
        assert_eq!(result.unwrap_err().slug(), "internal");
    }

    /// The session-state check (§16.6, option b): the alive session says
    /// true; the goTruth's 4xx (a dead session / a missing user) = NOT
    /// alive; the RateLimited and the unreachable network pass their
    /// error (fail-closed: the unchecked session NEVER passes).
    #[tokio::test]
    async fn the_session_check_decides_alive_dead_and_dependency() {
        // alive: the /user of a live session.
        let (base, _hits, _seen) = canned(move |_| {
            problem(
                StatusCode::OK,
                serde_json::json!({ "id": "3f2504e0-4f89-41d3-9a0c-0305e82c3301" }),
            )
        })
        .await;
        assert!(client(&base).session_alive("the-access").await.unwrap());

        // dead: any 4xx of goTruth.
        let (base, _hits, _seen) = canned(move |_| {
            problem(
                StatusCode::from_u16(401).expect("valid"),
                serde_json::json!({ "error": "invalid_grant", "error_description": "Invalid token" }),
            )
        })
        .await;
        assert!(!client(&base).session_alive("the-dead").await.unwrap());

        // the transient rate limit passes through (the caller renders it).
        let (base, _hits, _seen) = canned(move |_| {
            problem(
                StatusCode::from_u16(429).expect("valid"),
                serde_json::json!({ "error_code": "over_request_rate_limit" }),
            )
        })
        .await;
        let result = client(&base).session_alive("the-limited").await;
        assert_eq!(result.unwrap_err().slug(), "rate-limited");

        // the unreachable: fail-closed (503), never a pass.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the ephemeral port opens");
        let address = listener.local_addr().expect("the bind keeps it");
        drop(listener);
        let result = client(&format!("http://{address}"))
            .session_alive("the-unchecked")
            .await;
        assert_eq!(result.unwrap_err().slug(), "dependency-unavailable");
    }
}
