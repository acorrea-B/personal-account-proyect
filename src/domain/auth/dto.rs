//! The request DTOs of the auth flow (T012): the contract shapes of
//! `contracts/http-auth.md` with the validation of the catalogue (R4 and
//! email format) attached to the struct.
//!
//! The responses live in `handlers.rs` (they are its output, not its input).

use serde::Deserialize;
use validator::Validate;
use validator::ValidationError;

/// `POST /v1/auth/register` (FR-001): the plaintext password travels as
/// received (WITHOUT trim — the contract fixes it); the email normalizes in
/// the handler (trim + lowercase FR-002) BEFORE the validation runs.
#[derive(Debug, Deserialize, Validate)]
pub struct RegisterRequest {
    #[validate(email)]
    pub email: String,
    #[validate(custom(function = "password_policy"))]
    pub password: String,
}

/// `POST /v1/auth/resend-verification` (FR-005): the same normalization of
/// the email; the response is generic ALWAYS (FR-006) — the handler decides.
#[derive(Debug, Deserialize, Validate)]
pub struct ResendRequest {
    #[validate(email)]
    pub email: String,
}

/// R4 (the published requirements of the catalogue): length ≥ 8, ≥ 1 letter,
/// ≥ 1 number. The password never trims or transforms.
pub fn password_policy(password: &str) -> Result<(), ValidationError> {
    let long_enough = password.chars().count() >= 8;
    let letter = password.chars().any(char::is_alphabetic);
    let number = password.chars().any(char::is_numeric);
    if long_enough && letter && number {
        Ok(())
    } else {
        Err(ValidationError::new("weak_password"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R4 accepted: length ≥ 8, with letter and number; the spaces are part
    /// of the password (it never trims).
    #[test]
    fn the_password_policy_accepts_the_published_requirements() {
        for password in ["contraseña1", "abcdefgh 1", "a1b2c3d4"] {
            assert!(
                password_policy(password).is_ok(),
                "the password {password} meets the R4 requirements"
            );
        }
    }

    /// R4 rejected: each requirement fails alone — the error is of the
    /// password field (the handler maps it to `WeakPassword`).
    #[test]
    fn the_password_policy_rejects_each_requirement_alone() {
        let mut request = RegisterRequest {
            email: "juan@example.com".to_string(),
            password: "short".to_string(),
        };
        assert!(request
            .validate()
            .is_err_and(|errors| { errors.field_errors().contains_key("password") }));

        // Without number but enough length.
        request.password = "solo letras".to_string();
        assert!(request
            .validate()
            .is_err_and(|errors| { errors.field_errors().contains_key("password") }));

        // Without letter but enough length.
        request.password = "12345678".to_string();
        assert!(request
            .validate()
            .is_err_and(|errors| { errors.field_errors().contains_key("password") }));
    }

    /// The email format: malformed rejects under the email field (the
    /// handler maps it to `email-malformed` — 422 of the catalogue).
    #[test]
    fn a_malformed_email_rejects_under_its_field() {
        let request = RegisterRequest {
            email: "no-es-un-email".to_string(),
            password: "contraseña1".to_string(),
        };
        assert!(request
            .validate()
            .is_err_and(|errors| { errors.field_errors().contains_key("email") }));
    }
}
