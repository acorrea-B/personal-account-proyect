//! The handlers of the register flow (T012, US1): `register` (FR-001..003)
//! and `resend_verification` (FR-005/006), against the client T008 with the
//! events of D18 and the responses of the contract `http-auth.md`.
//!
//! The login/refresh/revoke handlers land with T015 — the router keeps
//! their stub until then.

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use uuid::Uuid;
use validator::Validate;

use crate::clients::supabase_auth::Session;
use crate::domain::auth::dto::{RegisterRequest, ResendRequest};
use crate::domain::auth::dto_session::{LoginRequest, RefreshRequest};
use crate::errors::{ApiError, Problem};
use crate::events::{Event, Outcome, Taxonomy};
use crate::state::AppState;

/// The public face of the user (the contract of register FR-003): never
/// credentials, never the internal columns of `auth.users` — id, email
/// (normalized), the verification state and the ISO8601 instant of creation.
#[derive(Debug, Serialize)]
pub struct UserPublic {
    pub id: Uuid,
    pub email: String,
    /// Register always answers `false` (R5 sent the verification email);
    /// the confirmation flips it in the flow of the trigger FR-004.
    pub verified: bool,
    pub created_at: String,
}

/// 201 of register (the contract): the user only.
#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub user: UserPublic,
}

/// The request id the access log will correlate (D18): the header the
/// middleware stamped or a fresh one — the same resolution the layer does.
fn request_id_of(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}

/// The body rejected by the extractor (structurally invalid JSON): the
/// canonical 400 of the contract, never a text of axum.
fn bad_request(body: JsonRejection, path: &str) -> Response {
    tracing::warn!(reason = "bad-request", path = %path, %body);
    Problem::new(ApiError::BadRequest {
        detail: "The body is not the JSON of the contract.".to_string(),
    })
    .on(path)
    .into_response()
}

/// `POST /v1/auth/register` (FR-001..003): normalizes the email, validates
/// and hands the signup to Supabase. The 201 never travels with credentials
/// — the session flow (login/refresh) is a separate contract (T015).
pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<RegisterRequest>, JsonRejection>,
) -> Response {
    const PATH: &str = "/v1/auth/register";

    let mut request = match body {
        Ok(Json(request)) => request,
        Err(rejection) => return bad_request(rejection, PATH),
    };

    // FR-002: the normalization happens BEFORE the validation (the spaces
    // and the case never reach the catalogue as malformation).
    request.email = request.email.trim().to_lowercase();

    if let Err(errors) = request.validate() {
        return Problem::new(field_error_of(&errors))
            .on(PATH)
            .into_response();
    }

    // The email escapes its struct: the signup and the 201 share it.
    let email = std::mem::take(&mut request.email);

    match state.supabase_auth.signup(&email, &request.password).await {
        Ok(user_id) => {
            Event::new(
                Taxonomy::AuthRegistered,
                Outcome::Accepted,
                request_id_of(&headers),
                Some(user_id.to_string()),
                None,
                Event::no_meta(),
            )
            .emit();

            // R5: the verification email always travels — `verified` is
            // deterministic in register; the client resolves it with its own
            // instant of creation (± the ms of this response).
            let instant = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
            (
                StatusCode::CREATED,
                Json(RegisterResponse {
                    user: UserPublic {
                        id: user_id,
                        email,
                        verified: false,
                        created_at: instant,
                    },
                }),
            )
                .into_response()
        }
        Err(error) => Problem::new(error).on(PATH).into_response(),
    }
}

/// `POST /v1/auth/resend-verification` (FR-005): the generic answer travels
/// ALWAYS (FR-006 — it never reveals existence); only the malformed email
/// and the real dependencies answer with a problem: every other Supabase
/// rejection (an account that does not exist, unmapped codes) resolves
/// generic — the leak of existence is the attack, the contract fixes it.
pub async fn resend_verification(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<ResendRequest>, JsonRejection>,
) -> Response {
    const PATH: &str = "/v1/auth/resend-verification";
    const GENERIC_OK: &str = "Si el email corresponde a una cuenta pendiente, se envió un enlace.";

    let request = match body {
        Ok(Json(request)) => request,
        Err(rejection) => return bad_request(rejection, PATH),
    };

    // The FR-002 normalization of the register applies here too (the same
    // account travels to the same email) and the validation runs on the
    // normalized struct — the call below only borrows it.
    let mut request = request;
    request.email = request.email.trim().to_lowercase();

    if let Err(errors) = request.validate() {
        return Problem::new(field_error_of(&errors))
            .on(PATH)
            .into_response();
    }

    match state
        .supabase_auth
        .resend_verification(&request.email)
        .await
    {
        Ok(()) => {
            Event::new(
                Taxonomy::AuthVerificationSent,
                Outcome::Accepted,
                request_id_of(&headers),
                None,
                None,
                Event::no_meta(),
            )
            .emit();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "message": GENERIC_OK })),
            )
                .into_response()
        }
        // The 422/429/503 of the catalogue pass through with their problem;
        // the rest (an email without account, unmapped codes) NEVER deny —
        // FR-006: the generic answer always travels.
        Err(
            error @ (ApiError::EmailMalformed
            | ApiError::RateLimited { .. }
            | ApiError::DependencyUnavailable
            | ApiError::BadRequest { .. }),
        ) => Problem::new(error).on(PATH).into_response(),
        Err(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "message": GENERIC_OK })),
        )
            .into_response(),
    }
}

/// The contract body of login/renewal (FR-006): the pair echoes verbatim,
/// the user in its public face. The `Session` of the client carries it.
fn session_response(session: &Session) -> serde_json::Value {
    serde_json::json!({
        "access_token": session.access_token,
        "refresh_token": session.refresh_token,
        "expires_in": session.expires_in,
        "user": {
            "id": session.user_id,
            "email": session.user_email,
            "verified": session.verified,
            "created_at": session.user_created_at,
        },
    })
}

/// `POST /v1/auth/login` (FR-006, US2): the normalized email travels to
/// Supabase — a wrong shape of credential is `invalid-credentials` (401),
/// identical and comparable in time for a bad password and for a
/// nonexistent email (SC-004, the client resolves the mapping). The pair
/// answers complete of the contract: tokens + TTL + the user.
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<LoginRequest>, JsonRejection>,
) -> Response {
    const PATH: &str = "/v1/auth/login";

    let mut request = match body {
        Ok(Json(request)) => request,
        Err(rejection) => return bad_request(rejection, PATH),
    };
    request.email = request.email.trim().to_lowercase();

    match state
        .supabase_auth
        .sign_in_with_password(&request.email, &request.password)
        .await
    {
        Ok(session) => {
            Event::new(
                Taxonomy::AuthLoginSucceeded,
                Outcome::Accepted,
                request_id_of(&headers),
                Some(session.user_id.to_string()),
                None,
                Event::no_meta(),
            )
            .emit();
            (StatusCode::OK, Json(session_response(&session))).into_response()
        }
        Err(error @ (ApiError::InvalidCredentials | ApiError::EmailNotVerified)) => {
            // The reason of the event is a catalogue slug (R9): the two
            // variants of this arm, verbatim — and the match does not
            // consume the error (unit variants).
            let reason = match error {
                ApiError::InvalidCredentials => "invalid-credentials",
                ApiError::EmailNotVerified => "email-not-verified",
                _ => unreachable!("the arm admits just these two variants"),
            };
            let mut meta = Event::no_meta();
            meta.insert(
                "reason".to_string(),
                serde_json::Value::String(reason.into()),
            );

            Event::new(
                Taxonomy::AuthLoginFailed,
                Outcome::Rejected,
                request_id_of(&headers),
                None,
                None,
                meta,
            )
            .emit();
            Problem::new(error).on(PATH).into_response()
        }
        Err(error) => Problem::new(error).on(PATH).into_response(),
    }
}

/// `POST /v1/auth/refresh` (FR-008, US2): the renewal rotates the pair —
/// the answer carries the NEW pair and the previous one stops serving; the
/// reuse (FR-008) invalidates the complete session at Supabase side.
pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<RefreshRequest>, JsonRejection>,
) -> Response {
    const PATH: &str = "/v1/auth/refresh";
    const REUSED: &str = "refresh-token-reused";
    const INVALID: &str = "invalid-refresh-token";

    let request = match body {
        Ok(Json(request)) => request,
        Err(rejection) => return bad_request(rejection, PATH),
    };

    match state.supabase_auth.refresh(&request.refresh_token).await {
        Ok(session) => {
            Event::new(
                Taxonomy::AuthSessionRefreshed,
                Outcome::Accepted,
                request_id_of(&headers),
                Some(session.user_id.to_string()),
                None,
                Event::no_meta(),
            )
            .emit();
            (StatusCode::OK, Json(session_response(&session))).into_response()
        }
        Err(error @ (ApiError::InvalidRefreshToken { .. } | ApiError::RefreshTokenReused)) => {
            // R9 again: the two reasons closed of `auth.refresh_rejected`.
            let reason = match error {
                ApiError::RefreshTokenReused => REUSED,
                ApiError::InvalidRefreshToken { .. } => INVALID,
                _ => unreachable!("the arm admits just these two variants"),
            };
            let mut meta = Event::no_meta();
            meta.insert(
                "reason".to_string(),
                serde_json::Value::String(reason.into()),
            );

            Event::new(
                Taxonomy::AuthRefreshRejected,
                Outcome::Rejected,
                request_id_of(&headers),
                None,
                None,
                meta,
            )
            .emit();
            Problem::new(error).on(PATH).into_response()
        }
        Err(error) => Problem::new(error).on(PATH).into_response(),
    }
}

/// The validation errors map to the catalogue: the email field →
/// `email-malformed`; the password → `weak-password` (R4, with requirements
/// in the extension of the document).
fn field_error_of(errors: &validator::ValidationErrors) -> ApiError {
    if errors.field_errors().contains_key("password") {
        ApiError::WeakPassword
    } else {
        ApiError::EmailMalformed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::supabase_auth::SupabaseAuthClient;
    use crate::domain::auth::dto::{password_policy, RegisterRequest, ResendRequest};
    use crate::middleware::hs256::Hs256Verifier;
    use crate::router::build_router;
    use axum::http::Request;
    use axum::routing::post as post_route;
    use axum::Router;
    use serde_json::Value;
    use sqlx::postgres::PgPoolOptions;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tower::ServiceExt;

    /// The answer default of the fake signup: the user with id (the shape
    /// of Supabase with the confirmation flow on — no session travels).
    const SIGNUP_USER: &str =
        r#"{"id": "3f2504e0-4f89-41d3-9a0c-0305e82c3301", "email": "juan@example.com"}"#;

    /// The fake of Supabase Auth: the same in-process server of the jwks
    /// tests (axum is a dependency of prod — no dev-dependency of mock http).
    /// The mutable status/body simulates the codes of Supabase; the signup
    /// and the resend share the state.
    type FakeSupabase = std::sync::Arc<Mutex<(u16, String)>>;

    fn set_response(fake: &FakeSupabase, status: u16, body: &str) {
        *fake.lock().expect("the fake never poisons") = (status, body.to_string());
    }

    async fn serve_supabase(State(fake): State<FakeSupabase>) -> (StatusCode, String) {
        let (status, body) = fake.lock().expect("the fake never poisons").clone();
        (
            StatusCode::from_u16(status).expect("the fake status is valid"),
            body,
        )
    }

    async fn spawn_fake_supabase() -> (FakeSupabase, String) {
        let fake: FakeSupabase = Arc::new(Mutex::new((200, SIGNUP_USER.to_string())));
        let app = Router::new()
            .route("/auth/v1/signup", post_route(serve_supabase))
            .route("/auth/v1/token", post_route(serve_supabase))
            .route("/auth/v1/resend", post_route(serve_supabase))
            .with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the ephemeral port opens");
        let url = format!(
            "http://{}",
            listener.local_addr().expect("the bind keeps it")
        );
        tokio::spawn(async move { axum::serve(listener, app).await.expect("the fake serves") });
        (fake, url)
    }

    /// A state like the production one (the client against the fake; the
    /// pool lazy — no DB is touched in these tests).
    async fn test_state() -> (AppState, FakeSupabase) {
        let (fake, url) = spawn_fake_supabase().await;
        let state = AppState {
            pool: PgPoolOptions::new()
                .connect_lazy("postgres://postgres:postgres@127.0.0.1:5432/postgres")
                .expect("the connection string is valid"),
            supabase_url: url.clone(),
            service_role_key: "test-service-role-key".to_string(),
            jwt_secret: String::new(),
            jwt_issuer: "supabase".to_string(),
            jwt_audience: "authenticated".to_string(),
            supabase_auth: SupabaseAuthClient::new(
                &url,
                "test-publishable",
                "test-service-role-key",
                Duration::from_secs(1),
            ),
            signature_verifier: Arc::new(Hs256Verifier::new(
                "test-jwt-secret",
                Some("supabase"),
                "authenticated",
            )),
        };
        (state, fake)
    }

    async fn body_of(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), 32 * 1024)
            .await
            .expect("the body reads");
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    async fn register_of(state: &AppState, email: &str, password: &str) -> (StatusCode, Value) {
        let response = register(
            State(state.clone()),
            HeaderMap::new(),
            Ok(Json(RegisterRequest {
                email: email.to_string(),
                password: password.to_string(),
            })),
        )
        .await;
        let status = response.status();
        (status, body_of(response).await)
    }

    /// The register of the contract: 201 with the normalized email, the
    /// `verified: false` deterministic, an ISO8601 instant and never
    /// credentials (FR-001..003, FR-002).
    #[tokio::test]
    async fn the_register_answers_the_201_of_the_contract_without_credentials() {
        let (state, _fake) = test_state().await;
        let (status, body) = register_of(&state, " JUAN@Example.com ", "contraseña1").await;

        assert_eq!(status, StatusCode::CREATED);
        let user = &body["user"];
        assert_eq!(user["email"], "juan@example.com", "FR-002 normalized");
        assert_eq!(user["verified"], false);
        chrono::DateTime::parse_from_rfc3339(
            user["created_at"]
                .as_str()
                .expect("instant of the contract"),
        )
        .expect("the instant is ISO8601");
        uuid::Uuid::parse_str(user["id"].as_str().expect("the id travels")).expect("uuid");
        assert!(body.get("access_token").is_none(), "no credentials travel");
        assert!(body.get("refresh_token").is_none(), "FR-003");
    }

    /// The email that already exists: the catalogue 409 (the client T008
    /// maps the code; the pass-through of the handler). Case-insensitive:
    /// the normalized email is the one that travels.
    #[tokio::test]
    async fn an_existing_email_resolves_the_409_of_the_catalogue() {
        let (state, fake) = test_state().await;
        set_response(&fake, 400, r#"{"error_code": "user_already_exists"}"#);

        let (status, body) = register_of(&state, "juan@example.com", "contraseña1").await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(body["type"]
            .as_str()
            .expect("type")
            .ends_with("email-exists"));
    }

    /// The weak password (R4): 422 with the requirements as extension — and
    /// without touching Supabase (the validation cuts before).
    #[tokio::test]
    async fn a_weak_password_answers_the_422_with_the_requirements() {
        let (state, _fake) = test_state().await;
        let (status, body) = register_of(&state, "juan@example.com", "solo-letras").await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body["type"]
            .as_str()
            .expect("type")
            .ends_with("weak-password"));
        let requirements = body["requirements"].to_string();
        assert!(
            requirements.contains("length"),
            "the R4 requirements travel"
        );
    }

    /// The malformed email: the 422 of the catalogue, before any network.
    #[tokio::test]
    async fn a_malformed_email_answers_the_422_before_the_network() {
        let (state, _fake) = test_state().await;
        let (status, body) = register_of(&state, "no-es-un-email", "contraseña1").await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body["type"]
            .as_str()
            .expect("type")
            .ends_with("email-malformed"));
    }

    /// The body that is not the JSON of the contract: the 400 canonical of
    /// the catalogue, never the plain text of axum (at the router level:
    /// the rejection of the extractor is of the wiring).
    #[tokio::test]
    async fn a_body_that_is_not_json_answers_the_400_of_the_catalogue() {
        let (state, _fake) = test_state().await;
        let router = build_router(state);

        let response = router
            .oneshot(
                Request::post("/v1/auth/register")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from("no-es-json"))
                    .expect("the request is valid"),
            )
            .await
            .expect("the router never fails");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// The resend of an email that does not exist: the 200 generic ALWAYS
    /// (FR-006 — the leak of existence is the attack; the unmapped codes of
    /// Supabase resolve the generic answer).
    #[tokio::test]
    async fn the_resend_of_an_unknown_email_answers_generic() {
        let (state, fake) = test_state().await;
        set_response(&fake, 400, r#"{"error_code": "user_not_found"}"#);

        let response = resend_verification(
            State(state.clone()),
            HeaderMap::new(),
            Ok(Json(ResendRequest {
                email: "desconocido@example.com".to_string(),
            })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "FR-006 never denies");
        let body = body_of(response).await;
        assert_eq!(
            body["message"],
            "Si el email corresponde a una cuenta pendiente, se envió un enlace."
        );
    }

    /// The malformed email of the resend: the only problem that the flow
    /// admits (422, before any network).
    #[tokio::test]
    async fn the_resend_of_a_malformed_email_answers_the_422() {
        let (state, _fake) = test_state().await;
        let response = resend_verification(
            State(state.clone()),
            HeaderMap::new(),
            Ok(Json(ResendRequest {
                email: "no-es-un-email".to_string(),
            })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body_of(response).await["type"]
            .as_str()
            .expect("type")
            .ends_with("email-malformed"));
    }

    /// The resend that Supabace accepted: 200 with the generic message of
    /// the contract (FR-005/006) and the event of the taxonomy.
    #[tokio::test]
    async fn the_accepted_resend_answers_the_generic_message_of_the_contract() {
        let (state, _fake) = test_state().await;
        let response = resend_verification(
            State(state.clone()),
            HeaderMap::new(),
            Ok(Json(ResendRequest {
                email: "juan@example.com".to_string(),
            })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_of(response).await["message"],
            "Si el email corresponde a una cuenta pendiente, se envió un enlace."
        );
    }

    /// The policy R4 as unit (the edge cases of the requirements alone).
    #[test]
    fn the_password_policy_of_the_catalogue_holds() {
        assert!(password_policy("contraseña1").is_ok());
        assert!(password_policy("short").is_err());
        assert!(password_policy("sin numeros").is_err());
        assert!(password_policy("12345678").is_err());
    }

    /// The pass-through arms of the flows: the client's OWN catalogue
    /// errors (not the validation cut) travel with their problem verbatim —
    /// the resend's `email-malformed` (rejected by Supabase itself), the
    /// login's 429 and the refresh's real dependency 503.
    #[tokio::test]
    async fn the_resend_of_an_email_rejected_by_the_platform_answers_the_422() {
        let (state, fake) = test_state().await;
        set_response(
            &fake,
            400,
            r#"{"error_code": "validation_failed", "msg": "Unable to validate email"}"#,
        );

        let response = resend_verification(
            State(state.clone()),
            HeaderMap::new(),
            Ok(Json(ResendRequest {
                email: "borrado-supabase@ejemplo.test".to_string(),
            })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body_of(response).await["type"]
            .as_str()
            .expect("type")
            .ends_with("email-malformed"));
    }

    #[tokio::test]
    async fn the_login_of_a_rate_limited_flow_answers_the_429_with_retry_after() {
        let (state, fake) = test_state().await;
        set_response(
            &fake,
            429,
            r#"{"error_code": "over_request_rate_limit", "msg": "Rate limit exceeded"}"#,
        );

        let (status, body) = login_of(&state, "juan@example.com", "contraseña1").await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert!(body["type"].as_str().expect("type").ends_with("rate-limited"));
        assert!(body["retry_after_seconds"].as_u64().unwrap_or(0) >= 1);
    }

    #[tokio::test]
    async fn the_refresh_of_a_dead_dependency_answers_the_503() {
        let (state, fake) = test_state().await;
        set_response(
            &fake,
            502,
            r#"{"error": "upstream", "error_description": "the platform is down"}"#,
        );

        let (status, body) = refresh_of(&state, "the-token").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(body["type"]
            .as_str()
            .expect("type")
            .ends_with("dependency-unavailable"));
    }

    /// The session answer the fake token grant carries: the pair + TTL +
    /// the confirmed user (verified true — the login of the confirmation).
    const TOKEN_BODY: &str = r#"{
        "access_token": "the-access", "refresh_token": "the-refresh",
        "expires_in": 3600,
        "user": {
            "id": "3f2504e0-4f89-41d3-9a0c-0305e82c3301",
            "email": "juan@example.com",
            "email_confirmed_at": "2026-10-23T00:00:00Z",
            "created_at": "2026-10-23T00:00:00Z"
        }
    }"#;

    async fn login_of(state: &AppState, email: &str, password: &str) -> (StatusCode, Value) {
        let response = login(
            State(state.clone()),
            HeaderMap::new(),
            Ok(Json(LoginRequest {
                email: email.to_string(),
                password: password.to_string(),
            })),
        )
        .await;
        let status = response.status();
        (status, body_of(response).await)
    }

    async fn refresh_of(state: &AppState, token: &str) -> (StatusCode, Value) {
        let response = refresh(
            State(state.clone()),
            HeaderMap::new(),
            Ok(Json(RefreshRequest {
                refresh_token: token.to_string(),
            })),
        )
        .await;
        let status = response.status();
        (status, body_of(response).await)
    }

    /// The login of the contract (FR-006): the complete pair — tokens, TTL
    /// (dynamic, not hardcoded from any side) and the user with the state
    /// of verification real of Supabase.
    #[tokio::test]
    async fn the_login_of_the_contract_carries_the_pair_and_the_user() {
        let (state, fake) = test_state().await;
        set_response(&fake, 200, TOKEN_BODY);

        let (status, body) = login_of(&state, "juan@example.com", "contraseña1").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["access_token"], "the-access");
        assert_eq!(body["refresh_token"], "the-refresh");
        assert_eq!(body["expires_in"], 3600);
        assert_eq!(body["user"]["email"], "juan@example.com");
        assert_eq!(body["user"]["verified"], true, "the confirmed user");
    }

    /// The failure of the catalog: incorrect credentials and nonexistent
    /// email resolve the same 401 (SC-004), and the arm emits `reason` (D18).
    #[tokio::test]
    async fn a_failed_login_answers_the_401_of_the_catalogue() {
        let (state, fake) = test_state().await;
        set_response(
            &fake,
            400,
            r#"{"error_code": "invalid_credentials", "msg": "Invalid login credentials"}"#,
        );

        let (status, body) = login_of(&state, "juan@example.com", "otra-pass").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body["type"]
            .as_str()
            .expect("type")
            .ends_with("invalid-credentials"));
    }

    /// The unconfirmed account: 403 with `can_resend` (the extension of the
    /// catalogue; the handler passes the 403 through with the extension
    /// built in errors.rs).
    #[tokio::test]
    async fn the_login_of_an_unconfirmed_account_answers_403_with_can_resend() {
        let (state, fake) = test_state().await;
        set_response(
            &fake,
            400,
            r#"{"error_code": "email_not_confirmed", "msg": "Email not confirmed"}"#,
        );

        let (status, body) = login_of(&state, "juan@example.com", "contraseña1").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body["type"]
            .as_str()
            .expect("type")
            .ends_with("email-not-verified"));
        assert_eq!(body["can_resend"], true);
    }

    /// The renewal rotates the pair: the response carries the NEW pair of
    /// the fixture and the user (FR-008).
    #[tokio::test]
    async fn the_renewal_answers_a_fresh_pair() {
        let (state, fake) = test_state().await;
        set_response(&fake, 200, TOKEN_BODY);

        let (status, body) = refresh_of(&state, "the-old-refresh").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["access_token"], "the-access");
        assert_eq!(body["refresh_token"], "the-refresh");
    }

    /// The reuse of a renewal credential: 401 of the catalogue (the client
    /// T008 maps `refresh_token_already_used` — the whole session dies at
    /// the Supabase side).
    #[tokio::test]
    async fn the_reuse_of_a_refresh_token_answers_the_rejection() {
        let (state, fake) = test_state().await;
        set_response(
            &fake,
            400,
            r#"{"error_code": "refresh_token_already_used", "msg": "Invalid Refresh Token"}"#,
        );

        let (status, body) = refresh_of(&state, "the-reused").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body["type"]
            .as_str()
            .expect("type")
            .ends_with("refresh-token-reused"));
    }

    /// A refresh credential that does not exist: `invalid-refresh-token`
    /// (401 — the contract; never the generic 500).
    #[tokio::test]
    async fn an_unknown_refresh_token_answers_invalid_refresh_token() {
        let (state, fake) = test_state().await;
        set_response(
            &fake,
            400,
            r#"{"error_code": "refresh_token_not_found", "msg": "Invalid Refresh Token"}"#,
        );

        let (status, body) = refresh_of(&state, "the-unknown").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body["type"]
            .as_str()
            .expect("type")
            .ends_with("invalid-refresh-token"));
    }
}
