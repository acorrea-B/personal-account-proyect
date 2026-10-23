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

use crate::domain::auth::dto::{RegisterRequest, ResendRequest};
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
