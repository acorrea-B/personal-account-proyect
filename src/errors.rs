//! Canonical catalogue of the API errors (RFC 7807 — D9/ADR 0009).
//!
//! Single source of the mapping slug ↔ status ↔ title of
//! `specs/001-beta-profile-api/contracts/errors.md`; the OpenAPI
//! documentation derives from here and the catalogue is closed: adding a
//! slug means one PR that adds the contract entry, the variant and the test
//! in the same pass.
//!
//! Every error answers `application/problem+json` with the fields `type`,
//! `title`, `status` and `detail` (English, per contract) plus the
//! extensions allowed by the table. No stack traces and no internal
//! technical messages ever (§13.4).

use std::collections::BTreeMap;

use axum::body::Body;
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::{Map, Value};

/// Base of the `type` URIs. The stable public domain of the namespace does
/// not exist yet (custom domain pending): the clients act on the slug of
/// the tail of the URI.
const TYPE_BASE: &str = "https://api.example.com";

/// Fixed generic detail of any `internal` (contract: fixed detail and
/// nothing internal leaks — §13.4).
pub const INTERNAL_DETAIL: &str = "An unexpected error occurred.";

/// Fixed detail of `invalid-credentials`: identical for a wrong password and
/// for a nonexistent email (FR-006 — no variant, no timing difference).
pub const INVALID_CREDENTIALS_DETAIL: &str = "Incorrect email or password.";

/// Fixed detail of `email-not-verified` (FR-003; the client may resend).
const EMAIL_NOT_VERIFIED_DETAIL: &str = "The email address is not verified yet.";

/// Fixed detail of `refresh-token-reused` (FR-008): the whole session is
/// invalidated by the one-use rotation of Supabase.
const REFRESH_TOKEN_REUSED_DETAIL: &str =
    "The refresh token was already used: the session was revoked.";

/// Fixed detail of `dependency-unavailable` (edge case of the spec).
const DEPENDENCY_UNAVAILABLE_DETAIL: &str =
    "The service is temporarily unavailable. Please retry later.";

/// The published password policy (R4): literals of the extension of
/// `weak-password` (already literals of the contract, not translated).
const PASSWORD_REQUIREMENTS: [&str; 3] = ["length ≥ 8", "≥ 1 letter", "≥ 1 number"];

/// The RFC 7807 document sent to the client.
///
/// The extension fields sit in a flattened map: the catalogue allows
/// different extensions per error and none of them is shared (`can_resend`
/// is exclusive of `email-not-verified`).
#[derive(Serialize)]
pub struct ProblemBody {
    r#type: String,
    title: String,
    status: u16,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instance: Option<String>,
    #[serde(flatten)]
    extensions: Map<String, Value>,
}

/// An `ApiError` together with the route that caused it (the `instance` of
/// the contract), assembled by the handler.
#[derive(Debug)]
pub struct Problem {
    error: ApiError,
    instance: Option<String>,
}

impl Problem {
    pub fn new(error: ApiError) -> Self {
        Self {
            error,
            instance: None,
        }
    }

    /// Binds the requested route as the `instance` of the document.
    pub fn on(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }
}

/// The HTTP contract errors (contract of `errors.md`, D9).
///
/// The variants without payload use the fixed detail of the catalogue; the
/// variants with payload describe the specific cause without leaking
/// internals.
#[derive(Debug)]
pub enum ApiError {
    /// Malformed body or structurally invalid JSON; also a syntactically
    /// invalid session credential when renewing.
    BadRequest { detail: String },
    /// Missing Authorization header or invalid credential.
    Unauthorized { detail: String },
    /// Expired credential of use (the client renews).
    TokenExpired { detail: String },
    /// Revoked session (logout / reuse detected).
    TokenRevoked { detail: String },
    /// Login: no variant — same detail for a wrong password and a
    /// nonexistent email (FR-006).
    InvalidCredentials,
    /// Renewal: nonexistent or expired refresh token.
    InvalidRefreshToken { detail: String },
    /// Rotation: reuse detected, the whole session invalidated (FR-008).
    RefreshTokenReused,
    /// Login with an unconfirmed account (FR-003). Extension `can_resend`.
    EmailNotVerified,
    /// Operation not allowed for the identity of the token.
    Forbidden { detail: String },
    /// Nonexistent route or own resource (e.g. a CV not saved yet).
    NotFound { detail: String },
    /// Registration with an already used email (case-insensitive, FR-002).
    EmailExists,
    /// Optimistic write: the resource changed since the last read of the
    /// client (contract `stale-version`, header `If-Unmodified-Since`).
    StaleVersion { current_updated_at: String },
    /// Content greater than 51200 bytes (FR-015).
    CvTooLarge { max_bytes: usize },
    /// Registration: the published requirements are not met (R4).
    WeakPassword,
    /// Registration/resend: an invalid email format.
    EmailMalformed,
    /// PUT profile: one or more invalid fields.
    ProfileInvalidField { fields: BTreeMap<String, String> },
    /// Text that is not valid UTF-8 (FR-015).
    CvInvalidEncoding,
    /// Limits R2 exceeded, with `Retry-After` (R2).
    RateLimited { retry_after_seconds: u64 },
    /// Unforeseen error — fixed generic detail.
    Internal,
    /// DB or Supabase without answer (edge case of the spec).
    DependencyUnavailable,
}

impl ApiError {
    /// The catalogue slug of this error (R9: the `meta.reason` of any event
    /// takes it, never another vocabulary).
    pub fn slug(self) -> &'static str {
        Catalogue::of(self).slug
    }
}

/// The catalogue entry of an error, resolved in one place.
// Centralization allowed by the rule of structure (§12.1): the whole
// mapping slug ↔ status ↔ title lives in one match, not in the handlers.
struct Catalogue {
    slug: &'static str,
    title: &'static str,
    status: StatusCode,
    detail: String,
    extensions: Map<String, Value>,
    retry_after_seconds: Option<u64>,
}

impl Catalogue {
    fn of(error: ApiError) -> Self {
        use ApiError::*;

        let (slug, title, status) = match &error {
            BadRequest { .. } => ("bad-request", "Bad Request", StatusCode::BAD_REQUEST),
            Unauthorized { .. } => ("unauthorized", "Unauthorized", StatusCode::UNAUTHORIZED),
            TokenExpired { .. } => ("token-expired", "Token Expired", StatusCode::UNAUTHORIZED),
            TokenRevoked { .. } => ("token-revoked", "Token Revoked", StatusCode::UNAUTHORIZED),
            InvalidCredentials => (
                "invalid-credentials",
                "Invalid Credentials",
                StatusCode::UNAUTHORIZED,
            ),
            InvalidRefreshToken { .. } => (
                "invalid-refresh-token",
                "Invalid Refresh Token",
                StatusCode::UNAUTHORIZED,
            ),
            RefreshTokenReused => (
                "refresh-token-reused",
                "Refresh Token Reused",
                StatusCode::UNAUTHORIZED,
            ),
            EmailNotVerified => (
                "email-not-verified",
                "Email Not Verified",
                StatusCode::FORBIDDEN,
            ),
            Forbidden { .. } => ("forbidden", "Forbidden", StatusCode::FORBIDDEN),
            NotFound { .. } => ("not-found", "Not Found", StatusCode::NOT_FOUND),
            EmailExists => (
                "email-exists",
                "Email Already Registered",
                StatusCode::CONFLICT,
            ),
            StaleVersion { .. } => (
                "stale-version",
                "Precondition Failed",
                StatusCode::PRECONDITION_FAILED,
            ),
            CvTooLarge { .. } => (
                "cv-too-large",
                "CV Too Large",
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
            WeakPassword => (
                "weak-password",
                "Weak Password",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            EmailMalformed => (
                "email-malformed",
                "Malformed Email",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            ProfileInvalidField { .. } => (
                "profile-invalid-field",
                "Profile Validation Failed",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            CvInvalidEncoding => (
                "cv-invalid-encoding",
                "CV Invalid Encoding",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            RateLimited { .. } => (
                "rate-limited",
                "Too Many Requests",
                StatusCode::TOO_MANY_REQUESTS,
            ),
            Internal => (
                "internal",
                "Internal Server Error",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            DependencyUnavailable => (
                "dependency-unavailable",
                "Service Unavailable",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
        };

        let mut extensions = Map::new();
        let mut retry_after_seconds = None;
        let detail: String = match error {
            BadRequest { detail } => detail,
            Unauthorized { detail } => detail,
            TokenExpired { detail } => detail,
            TokenRevoked { detail } => detail,
            InvalidCredentials => INVALID_CREDENTIALS_DETAIL.to_string(),
            InvalidRefreshToken { detail } => detail,
            RefreshTokenReused => REFRESH_TOKEN_REUSED_DETAIL.to_string(),
            EmailNotVerified => {
                extensions.insert("can_resend".to_string(), Value::Bool(true));
                EMAIL_NOT_VERIFIED_DETAIL.to_string()
            }
            Forbidden { detail } => detail,
            NotFound { detail } => detail,
            EmailExists => "The email address is already registered.".to_string(),
            StaleVersion { current_updated_at } => {
                extensions.insert(
                    "current_updated_at".to_string(),
                    Value::String(current_updated_at),
                );
                "The resource changed since your last read; nothing was overwritten.".to_string()
            }
            CvTooLarge { max_bytes } => {
                extensions.insert("max_bytes".to_string(), Value::from(max_bytes));
                "The CV content is too large.".to_string()
            }
            WeakPassword => {
                extensions.insert(
                    "requirements".to_string(),
                    Value::Array(
                        PASSWORD_REQUIREMENTS
                            .iter()
                            .map(|r| Value::String(r.to_string()))
                            .collect(),
                    ),
                );
                "The password does not meet the published requirements.".to_string()
            }
            EmailMalformed => "The email address is not well formed.".to_string(),
            ProfileInvalidField { fields } => {
                extensions.insert(
                    "fields".to_string(),
                    Value::Object(
                        fields
                            .into_iter()
                            .map(|(field, reason)| (field, Value::String(reason)))
                            .collect(),
                    ),
                );
                "One or more profile fields are invalid.".to_string()
            }
            CvInvalidEncoding => "The CV text is not valid UTF-8.".to_string(),
            RateLimited {
                retry_after_seconds: seconds,
            } => {
                extensions.insert("retry_after_seconds".to_string(), Value::from(seconds));
                retry_after_seconds = Some(seconds);
                "Too many requests; retry later.".to_string()
            }
            Internal => INTERNAL_DETAIL.to_string(),
            DependencyUnavailable => DEPENDENCY_UNAVAILABLE_DETAIL.to_string(),
        };

        Self {
            slug,
            title,
            status,
            detail,
            extensions,
            retry_after_seconds,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        Problem::new(self).into_response()
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let catalogue = Catalogue::of(self.error);

        let document = ProblemBody {
            r#type: format!("{TYPE_BASE}/errors/{}", catalogue.slug),
            title: catalogue.title.to_string(),
            status: catalogue.status.as_u16(),
            detail: catalogue.detail,
            instance: self.instance,
            extensions: catalogue.extensions,
        };

        let body = serde_json::to_vec(&document).expect("the problem body always serializes");
        let mut response = Response::new(Body::from(body));
        *response.status_mut() = catalogue.status;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );

        if let Some(seconds) = catalogue.retry_after_seconds {
            response.headers_mut().insert(
                HeaderName::from_static("retry-after"),
                HeaderValue::from_str(&seconds.to_string())
                    .expect("an u64 is a valid header value"),
            );
        }

        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    /// Body of a problem answer as JSON, for the assertions of the fields.
    async fn document(error: ApiError) -> Value {
        let response = Problem::new(error).into_response();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body is complete");
        serde_json::from_slice(&body).expect("a problem answer is always valid JSON")
    }

    /// Every variant of the catalogue maps to the status and the
    /// content-type of the contract; the slugs are asserted in the
    /// document test below.
    #[tokio::test]
    async fn every_error_maps_to_the_contract_status_and_content_type() {
        for (error, expected_status) in [
            (
                ApiError::BadRequest { detail: "".into() },
                StatusCode::BAD_REQUEST,
            ),
            (
                ApiError::Unauthorized { detail: "".into() },
                StatusCode::UNAUTHORIZED,
            ),
            (
                ApiError::TokenExpired { detail: "".into() },
                StatusCode::UNAUTHORIZED,
            ),
            (
                ApiError::TokenRevoked { detail: "".into() },
                StatusCode::UNAUTHORIZED,
            ),
            (ApiError::InvalidCredentials, StatusCode::UNAUTHORIZED),
            (
                ApiError::InvalidRefreshToken { detail: "".into() },
                StatusCode::UNAUTHORIZED,
            ),
            (ApiError::RefreshTokenReused, StatusCode::UNAUTHORIZED),
            (ApiError::EmailNotVerified, StatusCode::FORBIDDEN),
            (
                ApiError::Forbidden { detail: "".into() },
                StatusCode::FORBIDDEN,
            ),
            (
                ApiError::NotFound { detail: "".into() },
                StatusCode::NOT_FOUND,
            ),
            (ApiError::EmailExists, StatusCode::CONFLICT),
            (
                ApiError::StaleVersion {
                    current_updated_at: "2026-01-01T00:00:00Z".into(),
                },
                StatusCode::PRECONDITION_FAILED,
            ),
            (
                ApiError::CvTooLarge { max_bytes: 51200 },
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
            (ApiError::WeakPassword, StatusCode::UNPROCESSABLE_ENTITY),
            (ApiError::EmailMalformed, StatusCode::UNPROCESSABLE_ENTITY),
            (
                ApiError::ProfileInvalidField {
                    fields: BTreeMap::new(),
                },
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                ApiError::CvInvalidEncoding,
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                ApiError::RateLimited {
                    retry_after_seconds: 30,
                },
                StatusCode::TOO_MANY_REQUESTS,
            ),
            (ApiError::Internal, StatusCode::INTERNAL_SERVER_ERROR),
            (
                ApiError::DependencyUnavailable,
                StatusCode::SERVICE_UNAVAILABLE,
            ),
        ] {
            let response = Problem::new(error).into_response();
            assert_eq!(response.status(), expected_status);

            let content_type = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .expect("the problem answer always carries a content type");
            assert_eq!(content_type, "application/problem+json");
        }
    }

    /// The document carries the contract fields; the extensions of
    /// `email-not-verified` are `can_resend` and nothing else (rule 1).
    #[tokio::test]
    async fn email_not_verified_carries_exclusively_can_resend() {
        let value = document(ApiError::EmailNotVerified).await;
        assert_eq!(
            value["type"],
            format!("{TYPE_BASE}/errors/email-not-verified")
        );
        assert_eq!(value["title"], "Email Not Verified");
        assert_eq!(value["status"], 403);
        assert_eq!(value["detail"], EMAIL_NOT_VERIFIED_DETAIL);
        assert_eq!(value["can_resend"], true);

        // Rule 1: `can_resend` is exclusive of this slug.
        for slug in ["invalid-credentials", "internal", "forbidden"] {
            let other = document(match slug {
                "invalid-credentials" => ApiError::InvalidCredentials,
                "internal" => ApiError::Internal,
                _ => ApiError::Forbidden { detail: "".into() },
            })
            .await;
            assert!(
                other.get("can_resend").is_none(),
                "{slug} must not extend can_resend"
            );
            assert_eq!(other["type"], format!("{TYPE_BASE}/errors/{slug}"));
        }
    }

    /// The fixed details never change: `invalid-credentials` cannot be
    /// distinguishable (FR-006) and `internal` leaks nothing (§13.4).
    #[tokio::test]
    async fn the_fixed_details_are_the_contract_ones() {
        let credentials = document(ApiError::InvalidCredentials).await;
        assert_eq!(credentials["detail"], INVALID_CREDENTIALS_DETAIL);

        let internal: Value = document(ApiError::Internal).await;
        assert_eq!(internal["detail"], INTERNAL_DETAIL);
    }

    /// `rate-limited` answers with the extension and the header
    /// `Retry-After` (contract of the R2 limits).
    #[tokio::test]
    async fn rate_limited_carries_the_retry_after_information() {
        let response = Problem::new(ApiError::RateLimited {
            retry_after_seconds: 30,
        })
        .into_response();
        assert_eq!(
            response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok()),
            Some("30")
        );

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body is complete");
        let value: Value = serde_json::from_slice(&body).expect("valid JSON");
        assert_eq!(value["retry_after_seconds"], 30);
    }

    /// `weak-password` publishes the literal requirements of the contract.
    #[tokio::test]
    async fn weak_password_publishes_the_requirements() {
        let value = document(ApiError::WeakPassword).await;
        assert_eq!(
            value["requirements"],
            serde_json::json!(["length ≥ 8", "≥ 1 letter", "≥ 1 number"])
        );
    }

    /// `stale-version` names the current version so the client resyncs, and
    /// `profile-invalid-field` enumerates the failing fields.
    #[tokio::test]
    async fn the_stale_version_and_the_profile_field_extensions_travel() {
        let value = document(ApiError::StaleVersion {
            current_updated_at: "2026-01-01T00:00:00Z".into(),
        })
        .await;
        assert_eq!(value["current_updated_at"], "2026-01-01T00:00:00Z");

        let mut fields = BTreeMap::new();
        fields.insert("full_name".to_string(), "exceeds 200 chars".to_string());
        let value = document(ApiError::ProfileInvalidField { fields }).await;
        assert_eq!(value["fields"]["full_name"], "exceeds 200 chars");
    }

    /// The `instance` of the requested route is optional: absent by
    /// default, present after `Problem::on`.
    #[tokio::test]
    async fn the_instance_travel_only_when_bound() {
        let value = document(ApiError::NotFound { detail: "".into() }).await;
        assert!(value.get("instance").is_none());

        let response = Problem::new(ApiError::NotFound { detail: "".into() })
            .on("/v1/profile")
            .into_response();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body is complete");
        let value: Value = serde_json::from_slice(&body).expect("valid JSON");
        assert_eq!(value["instance"], "/v1/profile");
    }
}
