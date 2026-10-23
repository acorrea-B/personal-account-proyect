//! The request DTOs of the session flow (T015, US2): login and renewal —
//! the contract does not carry a 422 in them: the errors resolve to the
//! catalogue 401/429/503 (Supabase decides the shape of the credential).

use serde::Deserialize;

/// `POST /v1/auth/login` (FR-006): no validation of our own — a wrong shape
/// of credential is `invalid-credentials` (401) of Supabase, identical for a
/// bad password and for a nonexistent email (SC-004: comparable in time).
/// The email still normalizes (the same account key of FR-002).
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// `POST /v1/auth/refresh` (FR-008): the credential of renewal IS the
/// authentication — no header of ours carries identity.
#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}
